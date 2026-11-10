//! Runtime-registered state server behind the C ABI.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use parking_lot::{Mutex, RwLock};

use crate::hashing::{NoHashMap, generate_value_id};
use crate::protocol::check_value_size;
use crate::server_core::error::{CoreError, ErrorKind};
use crate::server_core::net::heartbeat::Heartbeat;
use crate::server_core::server::{Server, ServerConfig, StartError};
use crate::server_core::signals::{
    CLIENT_MESSAGE_ID, Claim, LOGGING_ID, ON_CONNECT_ID, ON_DISCONNECT_ID, SignalsManager,
};
use crate::server_core::states::data::{Data, DataMulti, DataSlice, header_type_id};
use crate::server_core::states::data_take::{DataMultiTake, DataTake};
use crate::server_core::states::image::common::checked_image_size;
use crate::server_core::states::image::multi::ImageMulti;
use crate::server_core::states::image::single::Image;
use crate::server_core::states::map::ValueMap;
use crate::server_core::states::value::{SignalCore, ValueCore, ValueStaticCore, ValueTakeCore};
use crate::server_core::states::vec::ValueList;
use crate::typed::{ObjectType, type_expression};

use super::error::{Error, ErrorCode, Result};
use super::events::ChangeEvent;
use super::kinds::{DataKind, StateKind};
use super::media::{ImageInput, data_slice};
use super::reader::Reader;
use super::tree::{Node, TypeRef};
use super::types::TypeRegistry;
use super::writer::Writer;

enum State {
    Value(Arc<ValueCore>, TypeRef),
    Static(Arc<ValueStaticCore>, TypeRef),
    ValueTake(Arc<ValueTakeCore>, TypeRef),
    Signal(Arc<SignalCore>, TypeRef),
    /// Core, whole `Vec<T>` type, and element type.
    Vec(Arc<ValueList>, TypeRef, TypeRef),
    /// Core, whole `HashMap<K, V>` type, key type, and value type.
    Map(Arc<ValueMap>, TypeRef, TypeRef, TypeRef),
    Image(Arc<Image>),
    ImageMulti(Arc<ImageMulti>),
    /// The Data kinds keep their element kind and header type.
    Data(Arc<Data>, DataKind, Option<TypeRef>),
    DataTake(Arc<DataTake>, DataKind, Option<TypeRef>),
    DataMulti(Arc<DataMulti>, DataKind, Option<TypeRef>),
    DataMultiTake(Arc<DataMultiTake>, DataKind, Option<TypeRef>),
    System(TypeRef),
}

struct Entry {
    name: String,
    /// Type expression reported by [`CServer::state_type`], if the state has one.
    type_text: Option<String>,
    state: State,
}

impl Entry {
    fn kind(&self) -> StateKind {
        match self.state {
            State::Value(..) => StateKind::Value,
            State::Static(..) => StateKind::Static,
            State::ValueTake(..) => StateKind::ValueTake,
            State::Signal(..) => StateKind::Signal,
            State::Vec(..) => StateKind::Vec,
            State::Map(..) => StateKind::Map,
            State::Image(..) => StateKind::Image,
            State::ImageMulti(..) => StateKind::ImageMulti,
            State::Data(..) => StateKind::Data,
            State::DataTake(..) => StateKind::DataTake,
            State::DataMulti(..) => StateKind::DataMulti,
            State::DataMultiTake(..) => StateKind::DataMultiTake,
            State::System(..) => StateKind::System,
        }
    }

    /// Type of the values delivered as events, for states that emit them.
    fn event_type(&self) -> Option<&TypeRef> {
        match &self.state {
            State::Value(_, value_type)
            | State::Signal(_, value_type)
            | State::System(value_type) => Some(value_type),
            _ => None,
        }
    }
}

struct States {
    types: TypeRegistry,
    entries: NoHashMap<u64, Entry>,
}

impl States {
    fn entry(&self, id: u64, expected: StateKind) -> Result<&Entry> {
        let entry = self.any_entry(id)?;
        let kind = entry.kind();
        if kind != expected {
            return Err(Error::type_mismatch(format!(
                "state '{}' is a {}, not a {}",
                entry.name,
                kind.name(),
                expected.name()
            )));
        }
        Ok(entry)
    }

    fn any_entry(&self, id: u64) -> Result<&Entry> {
        self.entries
            .get(&id)
            .ok_or_else(|| Error::not_found(format!("no state has id {id}")))
    }

    /// Returns the entry if it can deliver events.
    fn event_entry(&self, id: u64) -> Result<&Entry> {
        let entry = self.any_entry(id)?;
        if entry.event_type().is_none() {
            return Err(Error::type_mismatch(format!(
                "state '{}' is a {}; only Value, Signal, and system states deliver events",
                entry.name,
                entry.kind().name()
            )));
        }
        Ok(entry)
    }
}

/// Looks up a state of one kind and binds its fields, preceded by its name.
macro_rules! lookup {
    ($states:expr, $id:expr, $variant:ident($($binding:ident),*)) => {{
        let entry = $states.entry($id, StateKind::$variant)?;
        match &entry.state {
            State::$variant($($binding),*) => (entry.name.as_str(), $($binding),*),
            _ => return Err(Error::internal("state kind does not match its entry")),
        }
    }};
}

/// A state server whose states are registered at runtime from type strings.
///
/// Register states and type definitions, call [`Self::finalize`], then
/// [`Self::start`]. Every method is thread-safe. While a client is
/// connected, operations on `Data`, `DataMulti`, `Image`, `ImageMulti`, and
/// the take states may block until that state's previous transfer is
/// acknowledged or the client disconnects.
pub struct CServer {
    core: RwLock<Server>,
    signals: SignalsManager,
    /// Registered states and types until [`Self::finalize`] moves them to `frozen`.
    registration: Mutex<Option<States>>,
    frozen: OnceLock<States>,
}

impl CServer {
    /// Creates a server. With `version`, clients must present the same
    /// application version during the handshake.
    pub fn new(version: Option<u64>) -> Self {
        let core = Server::new(ServerConfig::with_version(version));
        let signals = core.get_signals_manager();

        let mut entries = NoHashMap::default();
        for (id, name, object_type) in [
            (
                LOGGING_ID,
                "logging",
                ObjectType::Tuple(vec![ObjectType::U8, ObjectType::String]),
            ),
            (ON_CONNECT_ID, "on_connect", ObjectType::String),
            (ON_DISCONNECT_ID, "on_disconnect", ObjectType::Empty),
            (CLIENT_MESSAGE_ID, "client_message", ObjectType::String),
        ] {
            entries.insert(
                id,
                Entry {
                    name: name.to_owned(),
                    type_text: Some(type_expression(&object_type)),
                    state: State::System(TypeRef::new(&object_type)),
                },
            );
        }

        Self {
            core: RwLock::new(core),
            signals,
            registration: Mutex::new(Some(States {
                types: TypeRegistry::default(),
                entries,
            })),
            frozen: OnceLock::new(),
        }
    }

    fn states(&self) -> Result<&States> {
        self.frozen
            .get()
            .ok_or_else(|| Error::invalid_state("the server is not finalized"))
    }

    // registration ----------------------------------------------------------

    /// Registers a `struct` or `enum` definition used by later type strings.
    pub fn define_type(&self, definition: &str) -> Result<()> {
        let mut registration = self.registration.lock();
        let states = registration.as_mut().ok_or_else(finalized)?;
        states.types.define(definition)
    }

    /// Returns the protocol hash of a type expression.
    pub fn type_hash(&self, type_text: &str) -> Result<u32> {
        let object_type = match self.frozen.get() {
            Some(states) => states.types.parse(type_text)?,
            None => match self.registration.lock().as_ref() {
                Some(states) => states.types.parse(type_text)?,
                // Finalization is in progress on another thread.
                None => self.states()?.types.parse(type_text)?,
            },
        };
        Ok(object_type.get_hash())
    }

    fn register(
        &self,
        name: &str,
        build: impl FnOnce(&TypeRegistry, &mut Server, u64) -> Result<(State, Option<String>)>,
    ) -> Result<u64> {
        let mut registration = self.registration.lock();
        let states = registration.as_mut().ok_or_else(finalized)?;
        if name.is_empty() {
            return Err(Error::invalid_argument("state names cannot be empty"));
        }
        let id = generate_value_id(name);
        if let Some(existing) = states.entries.get(&id) {
            let message = if existing.name == name {
                format!("state '{name}' is already registered")
            } else {
                format!("state '{name}' has the same id as '{}'", existing.name)
            };
            return Err(Error::new(ErrorCode::AlreadyExists, message));
        }

        let (state, type_text) = build(&states.types, &mut self.core.write(), id)?;
        states.entries.insert(
            id,
            Entry {
                name: name.to_owned(),
                type_text,
                state,
            },
        );
        Ok(id)
    }

    /// Registers a `Value` with an initial value; `None` is an empty writer.
    pub fn add_value(
        &self,
        name: &str,
        type_text: &str,
        initial: Option<&Writer>,
        queue: bool,
    ) -> Result<u64> {
        self.register(name, |types, core, id| {
            let (object_type, value_type) = parse(types, type_text)?;
            let data = sized(name, encode(initial, &value_type)?)?;
            core.add_value(name, object_type.get_hash(), data, queue)
                .map_err(Error::internal)?;
            let state = State::Value(registered(core.get_value(id))?, value_type);
            Ok((state, Some(type_expression(&object_type))))
        })
    }

    /// Registers a `Static` with an initial value; `None` is an empty writer.
    pub fn add_static(&self, name: &str, type_text: &str, initial: Option<&Writer>) -> Result<u64> {
        self.register(name, |types, core, id| {
            let (object_type, value_type) = parse(types, type_text)?;
            let data = sized(name, encode(initial, &value_type)?)?;
            core.add_static(name, object_type.get_hash(), data)
                .map_err(Error::internal)?;
            let state = State::Static(registered(core.get_static(id))?, value_type);
            Ok((state, Some(type_expression(&object_type))))
        })
    }

    /// Registers a `ValueTake`.
    pub fn add_value_take(&self, name: &str, type_text: &str) -> Result<u64> {
        self.register(name, |types, core, id| {
            let (object_type, value_type) = parse(types, type_text)?;
            core.add_value_take(name, object_type.get_hash())
                .map_err(Error::internal)?;
            let state = State::ValueTake(registered(core.get_value_take(id))?, value_type);
            Ok((state, Some(type_expression(&object_type))))
        })
    }

    /// Registers a `Signal`.
    pub fn add_signal(&self, name: &str, type_text: &str, queue: bool) -> Result<u64> {
        self.register(name, |types, core, id| {
            let (object_type, value_type) = parse(types, type_text)?;
            core.add_signal(name, object_type.get_hash(), queue)
                .map_err(Error::internal)?;
            let state = State::Signal(registered(core.get_signal(id))?, value_type);
            Ok((state, Some(type_expression(&object_type))))
        })
    }

    /// Registers a `VecState` with elements of `element_type`.
    pub fn add_vec(&self, name: &str, element_type: &str) -> Result<u64> {
        self.register(name, |types, core, id| {
            let element = types.parse(element_type)?;
            let hash = element.get_hash();
            let type_text = type_expression(&element);
            let whole = TypeRef::new(&ObjectType::Vec(Box::new(element)));
            let Node::Vec(item) = whole.tree.node(whole.node) else {
                return Err(Error::internal("vector type expected"));
            };
            let item = whole.at(item);
            core.add_vec(name, hash).map_err(Error::internal)?;
            let state = State::Vec(registered(core.get_vec(id))?, whole, item);
            Ok((state, Some(type_text)))
        })
    }

    /// Registers a `MapState` from `key_type` to `value_type`.
    pub fn add_map(&self, name: &str, key_type: &str, value_type: &str) -> Result<u64> {
        self.register(name, |types, core, id| {
            let key = types.parse(key_type)?;
            let value = types.parse(value_type)?;
            let hash = ObjectType::map_hash(&key, &value);
            let map_type = ObjectType::Map(Box::new(key), Box::new(value));
            let type_text = type_expression(&map_type);
            let whole = TypeRef::new(&map_type);
            let Node::Map(key, value) = whole.tree.node(whole.node) else {
                return Err(Error::internal("map type expected"));
            };
            let (key, value) = (whole.at(key), whole.at(value));
            core.add_map(name, hash).map_err(Error::internal)?;
            let state = State::Map(registered(core.get_map(id))?, whole, key, value);
            Ok((state, Some(type_text)))
        })
    }

    /// Registers an `Image`.
    pub fn add_image(&self, name: &str) -> Result<u64> {
        self.register(name, |_, core, id| {
            core.add_image(name).map_err(Error::internal)?;
            Ok((State::Image(registered(core.get_image(id))?), None))
        })
    }

    /// Registers an `ImageMulti`.
    pub fn add_image_multi(&self, name: &str) -> Result<u64> {
        self.register(name, |_, core, id| {
            core.add_image_multi(name).map_err(Error::internal)?;
            Ok((
                State::ImageMulti(registered(core.get_image_multi(id))?),
                None,
            ))
        })
    }

    /// Registers a `Data` buffer of `kind` elements, with a header of the type
    /// `header_type` names, or without one for `None`.
    pub fn add_data(&self, name: &str, kind: DataKind, header_type: Option<&str>) -> Result<u64> {
        self.register(name, |types, core, id| {
            let header = parse_header(types, header_type)?;
            core.add_data(
                name,
                kind as u8,
                header.as_ref().map(|(type_id, _)| *type_id),
            )
            .map_err(Error::internal)?;
            let header = header.map(|(_, header)| header);
            let state = State::Data(registered(core.get_data(id))?, kind, header);
            Ok((state, Some(kind.name().to_owned())))
        })
    }

    /// Registers a `DataTake` buffer of `kind` elements, with a header of the type
    /// `header_type` names, or without one for `None`.
    pub fn add_data_take(
        &self,
        name: &str,
        kind: DataKind,
        header_type: Option<&str>,
    ) -> Result<u64> {
        self.register(name, |types, core, id| {
            let header = parse_header(types, header_type)?;
            core.add_data_take(
                name,
                kind as u8,
                header.as_ref().map(|(type_id, _)| *type_id),
            )
            .map_err(Error::internal)?;
            let header = header.map(|(_, header)| header);
            let state = State::DataTake(registered(core.get_data_take(id))?, kind, header);
            Ok((state, Some(kind.name().to_owned())))
        })
    }

    /// Registers a `DataMulti` collection of `kind` buffers, with a header of the type
    /// `header_type` names, or without one for `None`.
    pub fn add_data_multi(
        &self,
        name: &str,
        kind: DataKind,
        header_type: Option<&str>,
    ) -> Result<u64> {
        self.register(name, |types, core, id| {
            let header = parse_header(types, header_type)?;
            core.add_data_multi(
                name,
                kind as u8,
                header.as_ref().map(|(type_id, _)| *type_id),
            )
            .map_err(Error::internal)?;
            let header = header.map(|(_, header)| header);
            let state = State::DataMulti(registered(core.get_data_multi(id))?, kind, header);
            Ok((state, Some(kind.name().to_owned())))
        })
    }

    /// Registers a `DataMultiTake` collection of `kind` buffers, with a header of the type
    /// `header_type` names, or without one for `None`.
    pub fn add_data_multi_take(
        &self,
        name: &str,
        kind: DataKind,
        header_type: Option<&str>,
    ) -> Result<u64> {
        self.register(name, |types, core, id| {
            let header = parse_header(types, header_type)?;
            core.add_data_multi_take(
                name,
                kind as u8,
                header.as_ref().map(|(type_id, _)| *type_id),
            )
            .map_err(Error::internal)?;
            let header = header.map(|(_, header)| header);
            let state =
                State::DataMultiTake(registered(core.get_data_multi_take(id))?, kind, header);
            Ok((state, Some(kind.name().to_owned())))
        })
    }

    /// Freezes registration. Calling it again has no effect.
    pub fn finalize(&self) -> Result<()> {
        let mut registration = self.registration.lock();
        let Some(states) = registration.take() else {
            return Ok(());
        };
        self.core.write().finalize();
        if self.frozen.set(states).is_err() {
            return Err(Error::internal("the server was finalized twice"));
        }
        Ok(())
    }

    // lifecycle -------------------------------------------------------------

    /// Sets the heartbeat used from the next [`Self::start`].
    ///
    /// The server pings the client every `interval` and disconnects it after
    /// `timeout` without any frame from it. A `None` timeout disables the
    /// heartbeat. The interval must be greater than zero and the timeout
    /// greater than the interval.
    pub fn set_heartbeat(&self, interval: Duration, timeout: Option<Duration>) -> Result<()> {
        let heartbeat = Heartbeat::new(interval, timeout).map_err(Error::invalid_argument)?;
        self.core.write().set_heartbeat(heartbeat);
        Ok(())
    }

    /// Sets which browser pages may connect from the next [`Self::start`],
    /// besides pages the server serves itself on an IP address or local
    /// network name.
    ///
    /// Each entry is a `scheme://host[:port]` pattern, where `*` matches any
    /// scheme, part of the host, or port; `*` alone allows every page. An
    /// empty list restores the default.
    pub fn set_allowed_origins(&self, origins: &[String]) -> Result<()> {
        self.core
            .write()
            .set_allowed_origins(origins)
            .map_err(Error::invalid_argument)
    }

    /// Starts listening on `port`, on the loopback address unless `ip` is given.
    ///
    /// `token` requires clients to present the same token. Starting a running
    /// server with the same settings succeeds without effect; a different
    /// address or token is an `InvalidState` error until the server is
    /// stopped.
    pub fn start(&self, port: u16, ip: Option<Ipv4Addr>, token: Option<String>) -> Result<()> {
        self.states()?;
        let address = SocketAddrV4::new(ip.unwrap_or(Ipv4Addr::LOCALHOST), port);
        self.core
            .read()
            .start(address, token)
            .map_err(|error| match error {
                StartError::InvalidState(message) => Error::new(ErrorCode::InvalidState, message),
                StartError::Io(message) => Error::new(ErrorCode::Io, message),
            })
    }

    /// Returns the address the server listens on, or `None` when stopped.
    pub fn address(&self) -> Option<SocketAddrV4> {
        self.core.read().address()
    }

    /// Stops listening and disconnects the client.
    pub fn stop(&self) {
        self.core.read().stop();
    }

    /// Disconnects the current client and keeps listening.
    pub fn disconnect_client(&self) {
        self.core.read().disconnect_client();
    }

    /// Returns whether the server is listening.
    pub fn is_running(&self) -> bool {
        self.core.read().is_running()
    }

    /// Returns whether a client is connected.
    pub fn is_connected(&self) -> bool {
        self.core.read().is_connected()
    }

    /// Asks the client to repaint after `delay` seconds (`0` for now).
    pub fn update(&self, delay: f32) -> Result<()> {
        if !delay.is_finite() || delay < 0.0 {
            return Err(Error::invalid_argument(format!(
                "the repaint delay must be finite and non-negative, got {delay}"
            )));
        }
        self.core
            .read()
            .update(Some(delay))
            .map_err(|_| Error::internal("failed to send the repaint request"))
    }

    // introspection ---------------------------------------------------------

    /// Returns the kind of the state with `id`.
    pub fn state_kind(&self, id: u64) -> Result<StateKind> {
        Ok(self.states()?.any_entry(id)?.kind())
    }

    /// Returns the full name of the state with `id`.
    pub fn state_name(&self, id: u64) -> Result<String> {
        Ok(self.states()?.any_entry(id)?.name.clone())
    }

    /// Returns the type expression of the state with `id`: the element type
    /// of vectors and data buffers, and `HashMap<K, V>` for maps.
    pub fn state_type(&self, id: u64) -> Result<String> {
        let entry = self.states()?.any_entry(id)?;
        entry.type_text.clone().ok_or_else(|| {
            Error::not_found(format!(
                "state '{}' is an image and has no value type",
                entry.name
            ))
        })
    }

    // values ----------------------------------------------------------------

    /// Loads the current value into `reader`.
    pub fn value_get(&self, id: u64, reader: &mut Reader) -> Result<()> {
        let (_, core, value_type) = lookup!(self.states()?, id, Value(core, value_type));
        reader.load(core.get(), value_type);
        Ok(())
    }

    /// Replaces the value; `signal` also delivers it to subscribers.
    pub fn value_set(&self, id: u64, writer: &Writer, signal: bool, update: bool) -> Result<()> {
        let (name, core, value_type) = lookup!(self.states()?, id, Value(core, value_type));
        let data = sized(name, writer.encoded(value_type)?)?;
        core.set(data, signal, update).map_err(Error::internal)
    }

    /// Loads the current static value into `reader`.
    pub fn static_get(&self, id: u64, reader: &mut Reader) -> Result<()> {
        let (_, core, value_type) = lookup!(self.states()?, id, Static(core, value_type));
        reader.load(core.get(), value_type);
        Ok(())
    }

    /// Replaces the static value.
    pub fn static_set(&self, id: u64, writer: &Writer, update: bool) -> Result<()> {
        let (name, core, value_type) = lookup!(self.states()?, id, Static(core, value_type));
        let data = sized(name, writer.encoded(value_type)?)?;
        core.set(data, update).map_err(Error::internal)
    }

    /// Sends a one-shot value; `None` is an empty writer.
    pub fn value_take_set(
        &self,
        id: u64,
        writer: Option<&Writer>,
        blocking: bool,
        update: bool,
    ) -> Result<()> {
        let (name, core, value_type) = lookup!(self.states()?, id, ValueTake(core, value_type));
        let data = sized(name, encode(writer, value_type)?)?;
        core.set(data, blocking, update).map_err(Error::internal)
    }

    /// Delivers a signal value to this server's subscribers; `None` is an
    /// empty writer.
    pub fn signal_emit(&self, id: u64, writer: Option<&Writer>) -> Result<()> {
        let (name, core, value_type) = lookup!(self.states()?, id, Signal(core, value_type));
        core.set(sized(name, encode(writer, value_type)?)?);
        Ok(())
    }

    // vectors ---------------------------------------------------------------

    /// Loads the whole vector into `reader`.
    pub fn vec_get(&self, id: u64, reader: &mut Reader) -> Result<()> {
        let (_, core, whole, _) = lookup!(self.states()?, id, Vec(core, whole, item));
        let items = core.get();
        let mut data = encode_len(items.len())?;
        for item in &items {
            data.extend_from_slice(item);
        }
        reader.load(Bytes::from(data), whole);
        Ok(())
    }

    /// Replaces the whole vector.
    pub fn vec_set(&self, id: u64, writer: &Writer, update: bool) -> Result<()> {
        let (name, core, whole, _) = lookup!(self.states()?, id, Vec(core, whole, item));
        let items = writer.items(whole)?;
        for item in &items {
            check_size(name, item.len())?;
        }
        core.set(items, update).map_err(Error::internal)
    }

    /// Loads one element into `reader`.
    pub fn vec_get_item(&self, id: u64, index: usize, reader: &mut Reader) -> Result<()> {
        let (_, core, _, item) = lookup!(self.states()?, id, Vec(core, whole, item));
        let data = core.get_item(index).map_err(out_of_range)?;
        reader.load(data, item);
        Ok(())
    }

    /// Replaces one element.
    pub fn vec_set_item(&self, id: u64, index: usize, writer: &Writer, update: bool) -> Result<()> {
        let (name, core, _, item) = lookup!(self.states()?, id, Vec(core, whole, item));
        let data = sized(name, writer.encoded(item)?)?;
        core.set_item_py(index, data, update)
            .map_err(Error::out_of_range)
    }

    /// Appends one element.
    pub fn vec_append(&self, id: u64, writer: &Writer, update: bool) -> Result<()> {
        let (name, core, _, item) = lookup!(self.states()?, id, Vec(core, whole, item));
        let data = sized(name, writer.encoded(item)?)?;
        core.append_item(data, update).map_err(Error::internal)
    }

    /// Removes one element, loading it into `removed` when given.
    pub fn vec_remove(
        &self,
        id: u64,
        index: usize,
        update: bool,
        removed: Option<&mut Reader>,
    ) -> Result<()> {
        let (_, core, _, item) = lookup!(self.states()?, id, Vec(core, whole, item));
        let data = core.remove_item(index, update).map_err(out_of_range)?;
        if let Some(reader) = removed {
            reader.load(data, item);
        }
        Ok(())
    }

    /// Returns the number of elements.
    pub fn vec_len(&self, id: u64) -> Result<usize> {
        let (_, core, _, _) = lookup!(self.states()?, id, Vec(core, whole, item));
        Ok(core.len())
    }

    // maps ------------------------------------------------------------------

    /// Loads the whole map into `reader`, in unspecified entry order.
    pub fn map_get(&self, id: u64, reader: &mut Reader) -> Result<()> {
        let (_, core, whole, _, _) = lookup!(self.states()?, id, Map(core, whole, key, value));
        let map = core.get();
        let mut data = encode_len(map.len())?;
        for (key, value) in &map {
            data.extend_from_slice(key);
            data.extend_from_slice(value);
        }
        reader.load(Bytes::from(data), whole);
        Ok(())
    }

    /// Replaces the whole map. Keys must be unique.
    pub fn map_set(&self, id: u64, writer: &Writer, update: bool) -> Result<()> {
        let (name, core, whole, _, _) = lookup!(self.states()?, id, Map(core, whole, key, value));
        let entries = writer.entries(whole)?;
        let mut map = HashMap::with_capacity(entries.len());
        for (key, value) in entries {
            check_size(name, key.len() + value.len())?;
            map.insert(key, value);
        }
        core.set(map, update).map_err(Error::internal)
    }

    /// Loads the value stored under `key` into `reader`.
    pub fn map_get_item(&self, id: u64, key: &Writer, reader: &mut Reader) -> Result<()> {
        let (name, core, _, key_type, value_type) =
            lookup!(self.states()?, id, Map(core, whole, key, value));
        let key = key.encoded(key_type)?;
        let value = core.get_item(&key).ok_or_else(|| missing_key(name))?;
        reader.load(value, value_type);
        Ok(())
    }

    /// Inserts or replaces the value stored under `key`.
    pub fn map_set_item(&self, id: u64, key: &Writer, value: &Writer, update: bool) -> Result<()> {
        let (name, core, _, key_type, value_type) =
            lookup!(self.states()?, id, Map(core, whole, key, value));
        let key = key.encoded(key_type)?;
        let value = value.encoded(value_type)?;
        check_size(name, key.len() + value.len())?;
        core.set_item(key, value, update).map_err(Error::internal)
    }

    /// Removes the entry under `key`, loading its value into `removed` when given.
    pub fn map_remove(
        &self,
        id: u64,
        key: &Writer,
        update: bool,
        removed: Option<&mut Reader>,
    ) -> Result<()> {
        let (name, core, _, key_type, value_type) =
            lookup!(self.states()?, id, Map(core, whole, key, value));
        let key = key.encoded(key_type)?;
        let value = core
            .remove_item(&key, update)
            .map_err(|_| Error::internal("failed to send the map removal"))?
            .ok_or_else(|| missing_key(name))?;
        if let Some(reader) = removed {
            reader.load(value, value_type);
        }
        Ok(())
    }

    /// Returns the number of entries.
    pub fn map_len(&self, id: u64) -> Result<usize> {
        let (_, core, _, _, _) = lookup!(self.states()?, id, Map(core, whole, key, value));
        Ok(core.len())
    }

    // images ----------------------------------------------------------------

    /// Returns the image shape as `[height, width]`; `[0, 0]` before the
    /// first set.
    pub fn image_size(&self, id: u64) -> Result<[usize; 2]> {
        let (_, core) = lookup!(self.states()?, id, Image(core));
        Ok(core.get_size())
    }

    /// Copies the RGBA pixels into `out` when it is large enough.
    ///
    /// Returns the byte count needed, the `[height, width]` shape, and
    /// whether the pixels were copied.
    pub fn image_get(&self, id: u64, out: Option<&mut [u8]>) -> Result<(usize, [usize; 2], bool)> {
        let (_, core) = lookup!(self.states()?, id, Image(core));
        Ok(core.get_image(|(data, size)| (data.len(), *size, copy_into(data, out))))
    }

    /// Replaces the whole image.
    pub fn image_set(&self, id: u64, image: &ImageInput<'_>, update: bool) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, Image(core));
        core.set_image(image.image_data()?, update)
            .map_err(Error::out_of_range)
    }

    /// Fills the image with one RGBA color, resizing it to `[height, width]`.
    pub fn image_set_all(
        &self,
        id: u64,
        size: [usize; 2],
        rgba: [u8; 4],
        update: bool,
    ) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, Image(core));
        checked_image_size(size).map_err(Error::out_of_range)?;
        core.set_all_image(size, rgba, update)
            .map_err(Error::out_of_range)
    }

    /// Replaces the rectangle at `origin` (`[y, x]`), which must lie inside
    /// the image. With `force`, it may replace a pending update of the same region.
    pub fn image_update(
        &self,
        id: u64,
        image: &ImageInput<'_>,
        origin: [usize; 2],
        update: bool,
        force: bool,
    ) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, Image(core));
        core.update_image(&origin, image.image_data()?, update, force)
            .map_err(Error::out_of_range)
    }

    /// Returns the shape of the image at `index` as `[height, width]`.
    pub fn image_multi_size(&self, id: u64, index: u32) -> Result<[usize; 2]> {
        let (name, core) = lookup!(self.states()?, id, ImageMulti(core));
        core.get_size(index)
            .ok_or_else(|| missing_index(name, index))
    }

    /// Copies the RGBA pixels at `index` into `out` when it is large enough.
    pub fn image_multi_get(
        &self,
        id: u64,
        index: u32,
        out: Option<&mut [u8]>,
    ) -> Result<(usize, [usize; 2], bool)> {
        let (name, core) = lookup!(self.states()?, id, ImageMulti(core));
        core.get_image(index, |image| match image {
            Some((data, size)) => Ok((data.len(), *size, copy_into(data, out))),
            None => Err(missing_index(name, index)),
        })
    }

    /// Replaces or creates the image at `index`.
    pub fn image_multi_set(
        &self,
        id: u64,
        index: u32,
        image: &ImageInput<'_>,
        update: bool,
    ) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        core.set_image(index, image.image_data()?, update)
            .map_err(Error::out_of_range)
    }

    /// Replaces or creates the image at `index`, filled with one RGBA color.
    pub fn image_multi_set_all(
        &self,
        id: u64,
        index: u32,
        size: [usize; 2],
        rgba: [u8; 4],
        update: bool,
    ) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        checked_image_size(size).map_err(Error::out_of_range)?;
        core.set_all_image(index, size, rgba, update)
            .map_err(Error::out_of_range)
    }

    /// Replaces a rectangle of the existing image at `index`.
    pub fn image_multi_update(
        &self,
        id: u64,
        index: u32,
        image: &ImageInput<'_>,
        origin: [usize; 2],
        update: bool,
        force: bool,
    ) -> Result<()> {
        let (name, core) = lookup!(self.states()?, id, ImageMulti(core));
        core.update_image(index, &origin, image.image_data()?, update, force)
            .map_err(keyed(name, index, Error::out_of_range))
    }

    /// Removes the image at `index`; a missing index is ignored.
    pub fn image_multi_remove_index(&self, id: u64, index: u32, update: bool) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        core.remove_index(index, update).map_err(Error::internal)
    }

    /// Removes every image.
    pub fn image_multi_reset(&self, id: u64, update: bool) -> Result<()> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        core.reset_images(update).map_err(Error::internal)
    }

    /// Returns the number of images.
    pub fn image_multi_len(&self, id: u64) -> Result<usize> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        Ok(core.len())
    }

    /// Returns whether an image exists at `index`.
    pub fn image_multi_contains(&self, id: u64, index: u32) -> Result<bool> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        Ok(core.contains(index))
    }

    /// Returns the indices of every image, in unspecified order.
    pub fn image_multi_indices(&self, id: u64) -> Result<Vec<u32>> {
        let (_, core) = lookup!(self.states()?, id, ImageMulti(core));
        Ok(core.indices())
    }

    // data ------------------------------------------------------------------

    /// Returns the number of elements.
    pub fn data_len(&self, id: u64) -> Result<usize> {
        let (_, core, kind, _header) = lookup!(self.states()?, id, Data(core, kind, header));
        Ok(core.get(|data| data.len() / kind.size()))
    }

    /// Copies the native-endian elements into `out` when it is large enough.
    ///
    /// Returns the element count and whether the elements were copied.
    pub fn data_get(
        &self,
        id: u64,
        kind: DataKind,
        out: Option<&mut [u8]>,
    ) -> Result<(usize, bool)> {
        let (name, core, registered, _header) =
            lookup!(self.states()?, id, Data(core, kind, header));
        check_kind(name, *registered, kind)?;
        Ok(core.get(|data| (data.len() / kind.size(), copy_into(data, out))))
    }

    /// Loads the header into `reader`; `NotFound` when the state has no
    /// header type or no header was set.
    pub fn data_get_header(&self, id: u64, reader: &mut Reader) -> Result<()> {
        let (name, core, _, header) = lookup!(self.states()?, id, Data(core, kind, header));
        let header_type = header_type(name, header)?;
        core.header(|bytes| load_header(name, bytes, header_type, reader))
    }

    // A data operation takes its new header as a writer; `None` keeps the
    // current one.

    /// Replaces the elements.
    pub fn data_set(
        &self,
        id: u64,
        kind: DataKind,
        data: &[u8],
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, Data(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.set(with_header(kind, data, &header)?, update)
            .map_err(Error::internal)
    }

    /// Appends elements.
    pub fn data_add(
        &self,
        id: u64,
        kind: DataKind,
        data: &[u8],
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, Data(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.add(with_header(kind, data, &header)?, update)
            .map_err(Error::internal)
    }

    /// Overwrites elements starting at element `index`.
    pub fn data_replace(
        &self,
        id: u64,
        kind: DataKind,
        data: &[u8],
        index: usize,
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, Data(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.replace(with_header(kind, data, &header)?, index, update)
            .map_err(Error::out_of_range)
    }

    /// Removes `count` elements starting at element `index`.
    pub fn data_remove(
        &self,
        id: u64,
        index: usize,
        count: usize,
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, _, header_type) = lookup!(self.states()?, id, Data(core, kind, header));
        check_count(count)?;
        let header = header_bytes(name, header_type, header)?;
        core.remove(index, count, header.as_deref(), update)
            .map_err(Error::out_of_range)
    }

    /// Removes every element; the header stays unless a new one is given.
    pub fn data_clear(&self, id: u64, header: Option<&Writer>, update: bool) -> Result<()> {
        let (name, core, _, header_type) = lookup!(self.states()?, id, Data(core, kind, header));
        let header = header_bytes(name, header_type, header)?;
        core.clear(header.as_deref(), update)
            .map_err(Error::internal)
    }

    /// Replaces the header alone; this never waits for the client.
    pub fn data_set_header(&self, id: u64, header: &Writer, update: bool) -> Result<()> {
        let (name, core, _, header_type) = lookup!(self.states()?, id, Data(core, kind, header));
        let header = header_bytes(name, header_type, Some(header))?.unwrap_or_default();
        core.set_header(&header, update).map_err(Error::internal)
    }

    /// Sends a one-shot buffer; `cache` keeps it for clients that connect
    /// later. Without a header, the client takes it with the default one.
    #[allow(clippy::too_many_arguments)]
    pub fn data_take_set(
        &self,
        id: u64,
        kind: DataKind,
        data: &[u8],
        header: Option<&Writer>,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, DataTake(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.set(with_header(kind, data, &header)?, blocking, update, cache)
            .map_err(Error::internal)
    }

    /// Copies the elements at `index` into `out` when it is large enough.
    pub fn data_multi_get(
        &self,
        id: u64,
        index: u32,
        kind: DataKind,
        out: Option<&mut [u8]>,
    ) -> Result<(usize, bool)> {
        let (name, core, registered, _header) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        check_kind(name, *registered, kind)?;
        core.get(index, |data| match data {
            Some(data) => Ok((data.len() / kind.size(), copy_into(data, out))),
            None => Err(missing_index(name, index)),
        })
    }

    /// Loads the header at `index` into `reader`; `NotFound` when the index
    /// is missing, the state has no header type or no header was set.
    pub fn data_multi_get_header(&self, id: u64, index: u32, reader: &mut Reader) -> Result<()> {
        let (name, core, _, header) = lookup!(self.states()?, id, DataMulti(core, kind, header));
        let header_type = header_type(name, header)?;
        core.header(index, |bytes| match bytes {
            Some(bytes) => load_header(name, bytes, header_type, reader),
            None => Err(missing_index(name, index)),
        })
    }

    /// Replaces or creates the buffer at `index`. A new index without a
    /// header gets the default one.
    pub fn data_multi_set(
        &self,
        id: u64,
        index: u32,
        kind: DataKind,
        data: &[u8],
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.set(index, with_header(kind, data, &header)?, update)
            .map_err(Error::internal)
    }

    /// Appends elements to the existing buffer at `index`.
    pub fn data_multi_add(
        &self,
        id: u64,
        index: u32,
        kind: DataKind,
        data: &[u8],
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.add(index, with_header(kind, data, &header)?, update)
            .map_err(keyed(name, index, Error::out_of_range))
    }

    /// Overwrites elements of the buffer at `index` starting at `data_index`.
    #[allow(clippy::too_many_arguments)]
    pub fn data_multi_replace(
        &self,
        id: u64,
        index: u32,
        kind: DataKind,
        data: &[u8],
        data_index: usize,
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.replace(index, data_index, with_header(kind, data, &header)?, update)
            .map_err(keyed(name, index, Error::out_of_range))
    }

    /// Removes `count` elements of the buffer at `index` starting at `data_index`.
    pub fn data_multi_remove(
        &self,
        id: u64,
        index: u32,
        data_index: usize,
        count: usize,
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, _, header_type) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        check_count(count)?;
        let header = header_bytes(name, header_type, header)?;
        core.remove(index, data_index, count, header.as_deref(), update)
            .map_err(keyed(name, index, Error::out_of_range))
    }

    /// Removes every element of the buffer at `index`, keeping the index and
    /// its header unless a new one is given.
    pub fn data_multi_clear(
        &self,
        id: u64,
        index: u32,
        header: Option<&Writer>,
        update: bool,
    ) -> Result<()> {
        let (name, core, _, header_type) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        let header = header_bytes(name, header_type, header)?;
        core.clear(index, header.as_deref(), update).map_err(keyed(
            name,
            index,
            Error::out_of_range,
        ))
    }

    /// Replaces the header at `index` alone; this never waits for the client.
    pub fn data_multi_set_header(
        &self,
        id: u64,
        index: u32,
        header: &Writer,
        update: bool,
    ) -> Result<()> {
        let (name, core, _, header_type) =
            lookup!(self.states()?, id, DataMulti(core, kind, header));
        let header = header_bytes(name, header_type, Some(header))?.unwrap_or_default();
        core.set_header(index, &header, update)
            .map_err(keyed(name, index, Error::internal))
    }

    /// Removes the buffer at `index`; a missing index is ignored.
    pub fn data_multi_remove_index(&self, id: u64, index: u32, update: bool) -> Result<()> {
        let (_, core, _, _header) = lookup!(self.states()?, id, DataMulti(core, kind, header));
        core.remove_index(index, update).map_err(Error::internal)
    }

    /// Removes every buffer.
    pub fn data_multi_reset(&self, id: u64, update: bool) -> Result<()> {
        let (_, core, _, _header) = lookup!(self.states()?, id, DataMulti(core, kind, header));
        core.reset(update).map_err(Error::internal)
    }

    /// Sends a one-shot buffer at `index`; without a header, the client
    /// takes it with the default one.
    #[allow(clippy::too_many_arguments)]
    pub fn data_multi_take_set(
        &self,
        id: u64,
        index: u32,
        kind: DataKind,
        data: &[u8],
        header: Option<&Writer>,
        blocking: bool,
        update: bool,
        cache: bool,
    ) -> Result<()> {
        let (name, core, registered, header_type) =
            lookup!(self.states()?, id, DataMultiTake(core, kind, header));
        check_kind(name, *registered, kind)?;
        let header = header_bytes(name, header_type, header)?;
        core.set(
            index,
            with_header(kind, data, &header)?,
            blocking,
            update,
            cache,
        )
        .map_err(Error::internal)
    }

    /// Removes the one-shot buffer at `index`; a missing index is ignored.
    pub fn data_multi_take_remove_index(&self, id: u64, index: u32, update: bool) -> Result<()> {
        let (_, core, _, _header) = lookup!(self.states()?, id, DataMultiTake(core, kind, header));
        core.remove_index(index, update).map_err(Error::internal)
    }

    /// Removes every one-shot buffer.
    pub fn data_multi_take_reset(&self, id: u64, update: bool) -> Result<()> {
        let (_, core, _, _header) = lookup!(self.states()?, id, DataMultiTake(core, kind, header));
        core.reset(update).map_err(Error::internal)
    }

    // events ----------------------------------------------------------------

    /// Starts delivering events of a `Value`, `Signal`, or system state;
    /// `with_previous` adds the replaced value to `Value` events.
    pub fn subscribe(&self, id: u64, with_previous: bool) -> Result<()> {
        self.states()?.event_entry(id)?;
        self.signals.set_register(id, true, with_previous);
        Ok(())
    }

    /// Stops delivering events of a state and drops its pending changes.
    pub fn unsubscribe(&self, id: u64) -> Result<()> {
        self.states()?.event_entry(id)?;
        self.signals.set_register(id, false, false);
        Ok(())
    }

    /// Selects queue mode, which delivers every change in order, or single
    /// mode, which coalesces pending changes to the latest.
    pub fn set_queue_mode(&self, id: u64, queue: bool) -> Result<()> {
        self.states()?.event_entry(id)?;
        if queue {
            self.signals.set_to_queue(id);
        } else {
            self.signals.set_to_single(id);
        }
        Ok(())
    }

    /// Claims the next change of a subscribed state, waiting until
    /// `deadline` (`None` waits indefinitely).
    pub fn wait_event(&self, deadline: Option<Instant>) -> Result<ChangeEvent> {
        let states = self.states()?;
        match self.signals.claim(None, deadline) {
            Claim::Ready(id, value, previous) => {
                let Some(value_type) = states.entries.get(&id).and_then(Entry::event_type) else {
                    self.signals.release(id);
                    return Err(Error::internal(format!(
                        "received an event of unknown state {id}"
                    )));
                };
                Ok(ChangeEvent::new(
                    self.signals.clone(),
                    id,
                    value,
                    previous,
                    value_type.clone(),
                ))
            }
            Claim::TimedOut => Err(Error::new(
                ErrorCode::Timeout,
                "no event arrived before the deadline",
            )),
            Claim::Closed => Err(Error::new(ErrorCode::Closed, "events are closed")),
        }
    }

    /// Wakes every waiter and makes current and future waits fail with
    /// [`ErrorCode::Closed`]. This cannot be undone.
    pub fn close_events(&self) {
        self.signals.close();
    }
}

impl Drop for CServer {
    fn drop(&mut self) {
        self.signals.close();
    }
}

fn finalized() -> Error {
    Error::invalid_state("states and types cannot be added after the server is finalized")
}

fn parse(types: &TypeRegistry, type_text: &str) -> Result<(ObjectType, TypeRef)> {
    let object_type = types.parse(type_text)?;
    let value_type = TypeRef::new(&object_type);
    Ok((object_type, value_type))
}

fn registered<T>(state: Option<Arc<T>>) -> Result<Arc<T>> {
    state.ok_or_else(|| Error::internal("state not found after registration"))
}

/// Parses a Data state's header type: its type id and type, or `None` for a
/// state without one. A type that describes no value, like `()`, means none.
fn parse_header(types: &TypeRegistry, header_type: Option<&str>) -> Result<Option<(u32, TypeRef)>> {
    let Some(header_type) = header_type else {
        return Ok(None);
    };
    let (object_type, header) = parse(types, header_type)?;
    Ok(header_type_id(&object_type).map(|type_id| (type_id, header)))
}

/// The header type of state `name`; `NotFound` without one.
fn header_type<'a>(name: &str, header: &'a Option<TypeRef>) -> Result<&'a TypeRef> {
    header
        .as_ref()
        .ok_or_else(|| Error::not_found(format!("'{name}' has no header type")))
}

/// Encodes the header in `writer` for state `name`, `None` without a writer.
fn header_bytes(
    name: &str,
    header_type: &Option<TypeRef>,
    writer: Option<&Writer>,
) -> Result<Option<Bytes>> {
    let Some(writer) = writer else {
        return Ok(None);
    };
    let Some(header_type) = header_type else {
        return Err(Error::invalid_argument(format!(
            "'{name}' was registered without a header"
        )));
    };
    let header = writer.encoded(header_type)?;
    check_size(&format!("{name} header"), header.len())?;
    Ok(Some(header))
}

/// The elements of `kind` in `data`, with the encoded `header`.
fn with_header<'a>(
    kind: DataKind,
    data: &'a [u8],
    header: &'a Option<Bytes>,
) -> Result<DataSlice<'a>> {
    Ok(DataSlice {
        header: header.as_deref(),
        ..data_slice(kind, data)?
    })
}

/// Loads a stored header into `reader`; `NotFound` when none was set.
fn load_header(
    name: &str,
    header: Option<&[u8]>,
    header_type: &TypeRef,
    reader: &mut Reader,
) -> Result<()> {
    let header = header.ok_or_else(|| Error::not_found(format!("'{name}' has no header yet")))?;
    reader.load(Bytes::copy_from_slice(header), header_type);
    Ok(())
}

/// Encodes an optional writer; a missing writer is an empty value.
fn encode(writer: Option<&Writer>, value_type: &TypeRef) -> Result<Bytes> {
    match writer {
        Some(writer) => writer.encoded(value_type),
        None => Writer::new().encoded(value_type),
    }
}

fn check_size(name: &str, size: usize) -> Result<()> {
    check_value_size(name, size).map_err(Error::out_of_range)
}

fn sized(name: &str, data: Bytes) -> Result<Bytes> {
    check_size(name, data.len())?;
    Ok(data)
}

fn encode_len(len: usize) -> Result<Vec<u8>> {
    postcard::to_stdvec(&(len as u64))
        .map_err(|error| Error::internal(format!("encoding failed: {error}")))
}

fn out_of_range(message: &str) -> Error {
    Error::out_of_range(message)
}

fn missing_key(name: &str) -> Error {
    Error::not_found(format!("the key is not in map '{name}'"))
}

fn missing_index(name: &str, index: u32) -> Error {
    Error::not_found(format!("'{name}' has nothing at index {index}"))
}

fn check_kind(name: &str, registered: DataKind, kind: DataKind) -> Result<()> {
    if registered != kind {
        return Err(Error::type_mismatch(format!(
            "'{name}' holds {} elements, not {}",
            registered.name(),
            kind.name()
        )));
    }
    Ok(())
}

fn check_count(count: usize) -> Result<()> {
    if count == 0 {
        return Err(Error::invalid_argument(
            "the element count must be at least 1",
        ));
    }
    Ok(())
}

/// Maps an error of a keyed core call: a key that is missing when the core
/// looks for it to [`missing_index`], anything else with `other`.
///
/// The core checks the key under its own lock; checking it here first could
/// race with a concurrent removal and report the wrong error.
fn keyed(name: &str, index: u32, other: fn(String) -> Error) -> impl Fn(CoreError) -> Error + '_ {
    move |error| match error.kind {
        ErrorKind::NotFound => missing_index(name, index),
        ErrorKind::Other => other(error.message),
    }
}

/// Copies `data` into `out` if it fits; `None` only asks for the size.
fn copy_into(data: &[u8], out: Option<&mut [u8]>) -> bool {
    match out {
        Some(out) if out.len() >= data.len() => {
            out[..data.len()].copy_from_slice(data);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn writer(write: impl FnOnce(&mut Writer) -> Result<()>) -> Writer {
        let mut writer = Writer::new();
        write(&mut writer).unwrap();
        writer
    }

    fn i32_writer(value: i32) -> Writer {
        writer(|w| w.i32(value))
    }

    fn read_i32(load: impl FnOnce(&mut Reader) -> Result<()>) -> i32 {
        let mut reader = Reader::new();
        load(&mut reader).unwrap();
        let value = reader.i32().unwrap();
        assert!(reader.at_end());
        value
    }

    fn soon() -> Option<Instant> {
        Some(Instant::now() + Duration::from_secs(2))
    }

    #[test]
    fn registration_follows_the_lifecycle() {
        let server = CServer::new(None);
        let error = server.value_get(1, &mut Reader::new()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidState);

        assert_eq!(
            server.add_value("", "i32", None, false).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let id = server
            .add_value("root.count", "i32", Some(&i32_writer(3)), false)
            .unwrap();
        assert_eq!(id, crate::c_abi::state_id("root.count"));
        let error = server.add_static("root.count", "i32", None).unwrap_err();
        assert_eq!(error.code, ErrorCode::AlreadyExists);
        assert_eq!(
            server
                .add_value("root.bad", "i32", None, false)
                .unwrap_err()
                .code,
            ErrorCode::TypeMismatch
        );
        assert_eq!(
            server
                .add_value("root.bad", "Vec<", None, false)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            server
                .add_value("root.bad", "Point", None, false)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        let large = writer(|w| w.string(&"x".repeat(2 * 1024 * 1024)));
        assert_eq!(
            server
                .add_value("root.large", "String", Some(&large), false)
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );

        server.finalize().unwrap();
        server.finalize().unwrap();
        assert_eq!(read_i32(|r| server.value_get(id, r)), 3);
        for error in [
            server.add_image("root.late").unwrap_err(),
            server.define_type("struct Late {}").unwrap_err(),
        ] {
            assert_eq!(error.code, ErrorCode::InvalidState);
        }
        assert_eq!(
            server.type_hash("Vec<u8>").unwrap(),
            <Vec<u8> as crate::Typed>::get_type().get_hash()
        );
    }

    #[test]
    fn lookups_report_unknown_ids_and_wrong_kinds() {
        let server = CServer::new(None);
        assert_eq!(
            server
                .add_static("root.label", "String", None)
                .unwrap_err()
                .code,
            ErrorCode::TypeMismatch
        );
        let id = server
            .add_static("root.label", "String", Some(&writer(|w| w.string("hi"))))
            .unwrap();
        server.finalize().unwrap();

        let error = server.value_get(id, &mut Reader::new()).unwrap_err();
        assert_eq!(error.code, ErrorCode::TypeMismatch);
        assert_eq!(error.message, "state 'root.label' is a Static, not a Value");
        let error = server.static_get(42, &mut Reader::new()).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);

        assert_eq!(server.state_kind(id).unwrap(), StateKind::Static);
        assert_eq!(server.state_name(id).unwrap(), "root.label");
        assert_eq!(server.state_type(id).unwrap(), "String");
        assert_eq!(
            server.state_kind(crate::c_abi::LOGGING_ID).unwrap(),
            StateKind::System
        );
        assert_eq!(
            server.state_type(crate::c_abi::LOGGING_ID).unwrap(),
            "(u8, String)"
        );
        let mut reader = Reader::new();
        server.static_get(id, &mut reader).unwrap();
        assert_eq!(reader.string().unwrap(), "hi");
    }

    #[test]
    fn values_and_collections_round_trip() {
        let server = CServer::new(None);
        server
            .define_type("struct Point { x: i32, label: String }")
            .unwrap();
        let value = server
            .add_value(
                "root.value",
                "Point",
                Some(&writer(|w| {
                    w.i32(1)?;
                    w.string("one")
                })),
                false,
            )
            .unwrap();
        let fixed = server
            .add_static(
                "root.fixed",
                "[u8; 2]",
                Some(&writer(|w| {
                    w.u8(1)?;
                    w.u8(2)
                })),
            )
            .unwrap();
        let take = server.add_value_take("root.take", "()").unwrap();
        let list = server.add_vec("root.list", "Option<u16>").unwrap();
        let map = server.add_map("root.map", "String", "i32").unwrap();
        server.finalize().unwrap();

        server
            .value_set(
                value,
                &writer(|w| {
                    w.i32(-5)?;
                    w.string("five")
                }),
                false,
                true,
            )
            .unwrap();
        let mut reader = Reader::new();
        server.value_get(value, &mut reader).unwrap();
        assert_eq!(reader.i32().unwrap(), -5);
        assert_eq!(reader.string().unwrap(), "five");
        assert!(
            server
                .value_set(value, &i32_writer(1), false, false)
                .is_err()
        );

        server
            .static_set(fixed, &writer(|w| w.array(DataKind::U8, &[7, 8], 2)), true)
            .unwrap();
        server.static_get(fixed, &mut reader).unwrap();
        assert_eq!((reader.u8().unwrap(), reader.u8().unwrap()), (7, 8));

        // Not connected, so neither take blocks.
        server.value_take_set(take, None, true, false).unwrap();
        assert!(
            server
                .value_take_set(take, Some(&i32_writer(1)), false, false)
                .is_err()
        );

        server
            .vec_set(
                list,
                &writer(|w| {
                    w.count(2)?;
                    w.some()?;
                    w.u16(10)?;
                    w.none()
                }),
                false,
            )
            .unwrap();
        server
            .vec_append(
                list,
                &writer(|w| {
                    w.some()?;
                    w.u16(30)
                }),
                false,
            )
            .unwrap();
        server
            .vec_set_item(
                list,
                1,
                &writer(|w| {
                    w.some()?;
                    w.u16(20)
                }),
                false,
            )
            .unwrap();
        assert_eq!(server.vec_len(list).unwrap(), 3);
        server.vec_get(list, &mut reader).unwrap();
        assert_eq!(reader.count().unwrap(), 3);
        for expected in [10, 20, 30] {
            assert!(reader.option().unwrap());
            assert_eq!(reader.u16().unwrap(), expected);
        }
        server
            .vec_remove(list, 0, false, Some(&mut reader))
            .unwrap();
        assert!(reader.option().unwrap());
        assert_eq!(reader.u16().unwrap(), 10);
        let error = server.vec_get_item(list, 2, &mut reader).unwrap_err();
        assert_eq!(error.code, ErrorCode::OutOfRange);
        assert_eq!(
            server
                .vec_set_item(list, 9, &writer(|w| w.none()), false)
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );

        let key = |text: &str| writer(|w| w.string(text));
        server
            .map_set(
                map,
                &writer(|w| {
                    w.count(2)?;
                    w.string("a")?;
                    w.i32(1)?;
                    w.string("b")?;
                    w.i32(2)
                }),
                false,
            )
            .unwrap();
        server
            .map_set_item(map, &key("c"), &i32_writer(3), false)
            .unwrap();
        assert_eq!(server.map_len(map).unwrap(), 3);
        assert_eq!(read_i32(|r| server.map_get_item(map, &key("b"), r)), 2);
        server
            .map_remove(map, &key("a"), false, Some(&mut reader))
            .unwrap();
        assert_eq!(reader.i32().unwrap(), 1);
        assert_eq!(
            server
                .map_get_item(map, &key("a"), &mut reader)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            server
                .map_remove(map, &key("a"), false, None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        server.map_get(map, &mut reader).unwrap();
        assert_eq!(reader.count().unwrap(), 2);
        let mut entries = Vec::new();
        for _ in 0..2 {
            entries.push((reader.string().unwrap().to_owned(), reader.i32().unwrap()));
        }
        entries.sort();
        assert_eq!(entries, [("b".to_owned(), 2), ("c".to_owned(), 3)]);
        let duplicate = writer(|w| {
            w.count(2)?;
            for _ in 0..2 {
                w.string("x")?;
                w.i32(0)?;
            }
            Ok(())
        });
        assert_eq!(
            server.map_set(map, &duplicate, false).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    /// A key that is missing when the core looks for it is NotFound with the
    /// C message, for every keyed call. The core checks the key under its own
    /// lock, so a concurrent removal cannot turn it into another error (D5).
    #[test]
    fn missing_keys_are_not_found_in_every_keyed_call() {
        let server = CServer::new(None);
        let images = server.add_image_multi("root.images").unwrap();
        let multi = server
            .add_data_multi("root.multi", DataKind::U8, None)
            .unwrap();
        server.finalize().unwrap();

        let patch = ImageInput {
            data: &[1],
            height: 1,
            width: 1,
            row_stride: 0,
            format: crate::c_abi::ImageFormat::Gray,
        };
        let results = [
            server.data_multi_add(multi, 3, DataKind::U8, &[1], None, false),
            server.data_multi_replace(multi, 3, DataKind::U8, &[1], 0, None, false),
            server.data_multi_remove(multi, 3, 0, 1, None, false),
            server.data_multi_clear(multi, 3, None, false),
            server.image_multi_update(images, 3, &patch, [0, 0], false, false),
        ];
        for result in results {
            let error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::NotFound, "{}", error.message);
            assert!(
                error.message.contains("has nothing at index 3"),
                "{}",
                error.message
            );
        }
    }

    #[test]
    fn images_and_data_round_trip() {
        let server = CServer::new(None);
        let image = server.add_image("root.image").unwrap();
        let images = server.add_image_multi("root.images").unwrap();
        let data = server.add_data("root.data", DataKind::U16, None).unwrap();
        let take = server
            .add_data_take("root.take", DataKind::U8, None)
            .unwrap();
        let multi = server
            .add_data_multi("root.multi", DataKind::F32, None)
            .unwrap();
        let multi_take = server
            .add_data_multi_take("root.multi_take", DataKind::I8, None)
            .unwrap();
        server.finalize().unwrap();

        assert_eq!(server.image_size(image).unwrap(), [0, 0]);
        let gray = [10u8, 20, 30, 40, 50, 60];
        let input = ImageInput {
            data: &gray,
            height: 2,
            width: 3,
            row_stride: 0,
            format: crate::c_abi::ImageFormat::Gray,
        };
        server.image_set(image, &input, false).unwrap();
        let (required, size, copied) = server.image_get(image, None).unwrap();
        assert_eq!((required, size, copied), (24, [2, 3], false));
        let mut pixels = [0u8; 24];
        assert!(server.image_get(image, Some(&mut pixels)).unwrap().2);
        assert_eq!(&pixels[..8], &[10, 10, 10, 255, 20, 20, 20, 255]);
        let patch = ImageInput {
            data: &[99],
            height: 1,
            width: 1,
            row_stride: 0,
            format: crate::c_abi::ImageFormat::Gray,
        };
        server
            .image_update(image, &patch, [1, 2], false, false)
            .unwrap();
        server.image_get(image, Some(&mut pixels)).unwrap();
        assert_eq!(&pixels[20..], &[99, 99, 99, 255]);
        assert_eq!(
            server
                .image_update(image, &patch, [2, 0], false, false)
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );
        server
            .image_set_all(image, [1, 1], [1, 2, 3, 4], false)
            .unwrap();
        assert_eq!(server.image_size(image).unwrap(), [1, 1]);
        assert_eq!(
            server
                .image_set_all(image, [0, 1], [0; 4], false)
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );

        server.image_multi_set(images, 4, &input, false).unwrap();
        server
            .image_multi_set_all(images, 9, [2, 2], [5; 4], false)
            .unwrap();
        assert_eq!(server.image_multi_len(images).unwrap(), 2);
        assert!(server.image_multi_contains(images, 9).unwrap());
        let mut indices = server.image_multi_indices(images).unwrap();
        indices.sort_unstable();
        assert_eq!(indices, [4, 9]);
        assert_eq!(server.image_multi_size(images, 4).unwrap(), [2, 3]);
        assert_eq!(
            server
                .image_multi_update(images, 7, &patch, [0, 0], false, false)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        server
            .image_multi_update(images, 4, &patch, [0, 0], false, false)
            .unwrap();
        server
            .image_multi_get(images, 4, Some(&mut pixels))
            .unwrap();
        assert_eq!(&pixels[..4], &[99, 99, 99, 255]);
        server.image_multi_remove_index(images, 4, false).unwrap();
        server.image_multi_remove_index(images, 4, false).unwrap();
        assert_eq!(
            server.image_multi_get(images, 4, None).unwrap_err().code,
            ErrorCode::NotFound
        );
        server.image_multi_reset(images, false).unwrap();
        assert_eq!(server.image_multi_len(images).unwrap(), 0);

        let raw = |values: &[u16]| {
            values
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect::<Vec<_>>()
        };
        server
            .data_set(data, DataKind::U16, &raw(&[1, 2, 3]), None, false)
            .unwrap();
        server
            .data_add(data, DataKind::U16, &raw(&[4]), None, false)
            .unwrap();
        server
            .data_replace(data, DataKind::U16, &raw(&[9]), 0, None, false)
            .unwrap();
        server.data_remove(data, 1, 1, None, false).unwrap();
        assert_eq!(server.data_len(data).unwrap(), 3);
        let mut out = [0u8; 6];
        assert_eq!(
            server
                .data_get(data, DataKind::U16, Some(&mut out))
                .unwrap(),
            (3, true)
        );
        assert_eq!(out.to_vec(), raw(&[9, 3, 4]));
        assert_eq!(
            server.data_get(data, DataKind::U32, None).unwrap_err().code,
            ErrorCode::TypeMismatch
        );
        assert_eq!(
            server
                .data_replace(data, DataKind::U16, &raw(&[1, 1]), 2, None, false)
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );
        assert_eq!(
            server
                .data_remove(data, 0, 0, None, false)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        server.data_clear(data, None, false).unwrap();
        assert_eq!(server.data_len(data).unwrap(), 0);

        server
            .data_take_set(take, DataKind::U8, &[1, 2], None, true, false, true)
            .unwrap();

        let floats = |values: &[f32]| {
            values
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            server
                .data_multi_add(multi, 3, DataKind::F32, &floats(&[1.0]), None, false)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        server
            .data_multi_set(multi, 3, DataKind::F32, &floats(&[1.0, 2.0]), None, false)
            .unwrap();
        server
            .data_multi_add(multi, 3, DataKind::F32, &floats(&[3.0]), None, false)
            .unwrap();
        server
            .data_multi_replace(multi, 3, DataKind::F32, &floats(&[-1.0]), 1, None, false)
            .unwrap();
        server
            .data_multi_remove(multi, 3, 0, 1, None, false)
            .unwrap();
        let mut out = [0u8; 8];
        assert_eq!(
            server
                .data_multi_get(multi, 3, DataKind::F32, Some(&mut out))
                .unwrap(),
            (2, true)
        );
        assert_eq!(out.to_vec(), floats(&[-1.0, 3.0]));
        server.data_multi_clear(multi, 3, None, false).unwrap();
        assert_eq!(
            server
                .data_multi_get(multi, 3, DataKind::F32, None)
                .unwrap(),
            (0, false)
        );
        server.data_multi_remove_index(multi, 3, false).unwrap();
        assert_eq!(
            server
                .data_multi_get(multi, 3, DataKind::F32, None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        server.data_multi_reset(multi, false).unwrap();

        server
            .data_multi_take_set(multi_take, 1, DataKind::I8, &[1], None, false, false, false)
            .unwrap();
        server
            .data_multi_take_remove_index(multi_take, 1, false)
            .unwrap();
        server.data_multi_take_reset(multi_take, false).unwrap();
    }

    #[test]
    fn events_deliver_subscribed_changes() {
        let server = CServer::new(None);
        let value = server
            .add_value("root.value", "i32", Some(&i32_writer(0)), false)
            .unwrap();
        let signal = server
            .add_signal("root.signal", "(u8, bool)", true)
            .unwrap();
        let fixed = server
            .add_static("root.fixed", "i32", Some(&i32_writer(0)))
            .unwrap();
        server.finalize().unwrap();

        assert_eq!(
            server.subscribe(fixed, false).unwrap_err().code,
            ErrorCode::TypeMismatch
        );
        server.subscribe(value, true).unwrap();
        server.subscribe(signal, false).unwrap();
        server
            .subscribe(crate::c_abi::ON_CONNECT_ID, false)
            .unwrap();

        // Single mode coalesces a -> b -> c and reports the value last delivered.
        for next in [1, 2, 3] {
            server
                .value_set(value, &i32_writer(next), true, false)
                .unwrap();
        }
        let event = server.wait_event(soon()).unwrap();
        assert_eq!(event.id(), value);
        assert_eq!(
            read_i32(|r| {
                event.load_value(r);
                Ok(())
            }),
            3
        );
        assert!(event.has_previous());
        assert_eq!(read_i32(|r| event.load_previous(r)), 0);

        // The claim is held until the event is dropped.
        server
            .value_set(value, &i32_writer(4), true, false)
            .unwrap();
        assert_eq!(
            server.wait_event(Some(Instant::now())).unwrap_err().code,
            ErrorCode::Timeout
        );
        drop(event);
        let event = server.wait_event(soon()).unwrap();
        assert_eq!(read_i32(|r| event.load_previous(r)), 3);
        drop(event);

        // Signals were registered in queue mode, so both emits are delivered.
        for number in [7, 8] {
            server
                .signal_emit(
                    signal,
                    Some(&writer(|w| {
                        w.u8(number)?;
                        w.bool(true)
                    })),
                )
                .unwrap();
        }
        for number in [7, 8] {
            let event = server.wait_event(soon()).unwrap();
            assert_eq!(event.id(), signal);
            assert!(!event.has_previous());
            let mut reader = Reader::new();
            event.load_value(&mut reader);
            assert_eq!(reader.u8().unwrap(), number);
            assert_eq!(
                event.load_previous(&mut reader).unwrap_err().code,
                ErrorCode::NotFound
            );
        }

        // Unsubscribed states and unflagged sets produce no events.
        server
            .value_set(value, &i32_writer(5), false, false)
            .unwrap();
        server.unsubscribe(signal).unwrap();
        server
            .signal_emit(
                signal,
                Some(&writer(|w| {
                    w.u8(1)?;
                    w.bool(false)
                })),
            )
            .unwrap();
        assert_eq!(
            server
                .wait_event(Some(Instant::now() + Duration::from_millis(20)))
                .unwrap_err()
                .code,
            ErrorCode::Timeout
        );

        server.set_queue_mode(value, true).unwrap();
        for next in [10, 11] {
            server
                .value_set(value, &i32_writer(next), true, false)
                .unwrap();
        }
        for (next, previous) in [(10, 5), (11, 10)] {
            let event = server.wait_event(soon()).unwrap();
            assert_eq!(
                read_i32(|r| {
                    event.load_value(r);
                    Ok(())
                }),
                next
            );
            assert_eq!(read_i32(|r| event.load_previous(r)), previous);
        }

        server.close_events();
        assert_eq!(server.wait_event(None).unwrap_err().code, ErrorCode::Closed);
    }

    #[test]
    fn start_requires_finalize_and_reports_bind_errors() {
        let server = CServer::new(None);
        assert_eq!(
            server.start(0, None, None).unwrap_err().code,
            ErrorCode::InvalidState
        );
        server.finalize().unwrap();
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let error = server
            .start(port, Some(Ipv4Addr::LOCALHOST), None)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Io);
        assert!(!server.is_running());
        assert_eq!(
            server.update(-1.0).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            server.update(f32::NAN).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        server.update(0.0).unwrap();
    }
}
