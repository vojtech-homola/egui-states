//! State-handle factories and stable state-layout hashing.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::messages::MessageSender;
use crate::client::states::atomics::Atomic;
use crate::client::states::data::{
    Data, DataMulti, UpdateData, UpdateMultiData, data_type_hash, private::GetDataType,
};
use crate::client::states::data_take::{
    DataMultiTake, DataTake, UpdateDataMultiTake, UpdateDataTake,
};
use crate::client::states::file_transfer::{Download, Upload};
use crate::client::states::header::HeaderValue;
use crate::client::states::image::{Image, ImageMulti};
use crate::client::states::initial_value::InitialValue;
use crate::client::states::map::{MapState, UpdateMap};
use crate::client::states::values::{
    Signal, Static, StaticAtomic, UpdateValue, UpdateValueTake, Value, ValueAtomic, ValueTake,
};
use crate::client::states::vec::{UpdateList, VecState};
use crate::file_transfer::engine::{Engine, Repaint, Role, Side};
use crate::file_transfer::{TransferMeta, meta_type_id};
use crate::hashing::{NoHashMap, StableHasher, generate_value_id};
use crate::typed::{ObjectType, Typed};

#[inline]
pub(crate) fn hash_id_type(hasher: &mut StableHasher, id: u64, type_id: u32, hash_id: u8) {
    id.hash(hasher);
    type_id.hash(hasher);
    hash_id.hash(hasher);
}

#[inline]
pub(crate) fn hash_id(hasher: &mut StableHasher, id: u64) {
    id.hash(hasher);
}

#[inline]
pub(crate) fn hash_id_kind(hasher: &mut StableHasher, id: u64, hash_id: u8) {
    id.hash(hasher);
    hash_id.hash(hasher);
}

/// Factory used by [`State::new`](crate::State::new) to register client states.
///
/// Applications normally receive an implementation through a derived
/// [`State`](derive@crate::State) implementation instead of implementing this
/// trait themselves. Every `name` becomes one segment below the current state
/// path (for example, `counter` below the root becomes `root.counter`) and must
/// be unique among siblings.
pub trait StatesCreator {
    /// Creates a nested state group under `name`.
    fn substate<S: State>(&mut self, name: &str) -> S;

    /// Creates a synchronized read/write value with an initial client value.
    fn value<T>(&mut self, name: &'static str, value: T) -> Value<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Send
            + Sync
            + Clone
            + 'static;

    /// Creates a one-shot value received by and consumed on the client.
    fn value_take<T>(&mut self, name: &'static str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Typed + Send + Sync + 'static;

    /// Creates an atomic synchronized read/write value.
    fn atomic<T>(&mut self, name: &'static str, value: T) -> ValueAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Send
            + Sync
            + Clone
            + Atomic
            + 'static;

    /// Creates a server-controlled value that is read-only on the client.
    fn add_static<T>(&mut self, name: &'static str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Clone
            + Send
            + Sync
            + 'static;

    /// Creates an atomic server-controlled value that is read-only on the client.
    fn static_atomic<T>(&mut self, name: &'static str, value: T) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Clone
            + Send
            + Sync
            + Atomic
            + 'static;

    /// Creates a client-to-server event carrying values of type `T`.
    fn signal<T>(&mut self, name: &'static str) -> Signal<T>
    where
        T: Serialize + Typed + Clone + Send + Sync + 'static;

    /// Creates a server-controlled egui image texture.
    fn image(&mut self, name: &'static str) -> Image;

    /// Creates server-controlled egui image textures indexed by `u32` keys.
    fn image_multi(&mut self, name: &'static str) -> ImageMulti;

    /// Creates a server-controlled key/value collection.
    fn map<K, V>(&mut self, name: &'static str) -> MapState<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + Typed + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + Typed + 'static;

    /// Creates a server-controlled vector collection.
    fn vec<T>(&mut self, name: &'static str) -> VecState<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + Typed + 'static;

    #[allow(private_bounds)]
    /// Creates a contiguous server-controlled numeric buffer, with the header type `H` (`()` for none).
    fn data<T, H>(&mut self, name: &'static str) -> Data<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue;

    #[allow(private_bounds)]
    /// Creates keyed server-controlled numeric buffers, with the header type `H` (`()` for none).
    fn data_multi<T, H>(&mut self, name: &'static str) -> DataMulti<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue;

    #[allow(private_bounds)]
    /// Creates a one-shot numeric buffer consumed by the client, with the header type `H` (`()` for none).
    fn data_take<T, H>(&mut self, name: &'static str) -> DataTake<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue;

    #[allow(private_bounds)]
    /// Creates keyed one-shot numeric buffers consumed by the client, with the header type `H` (`()` for none).
    fn data_multi_take<T, H>(&mut self, name: &'static str) -> DataMultiTake<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue;

    /// Creates an endpoint that receives files from the server, with the
    /// metadata type `M` (`()` for none).
    fn download<M: TransferMeta>(&mut self, name: &'static str) -> Download<M>;

    /// Creates an endpoint that sends files to the server, with the metadata
    /// type `M` (`()` for none).
    fn upload<M: TransferMeta>(&mut self, name: &'static str) -> Upload<M>;

    /// Creates the states that `f` makes with low send priority.
    ///
    /// The server sends their transfers in a lane of their own, only while no
    /// other state has a message waiting and a window of their data at a
    /// time, so small states do not wait behind them. In exchange, a change
    /// of another state made after a change of one of them can reach the
    /// client first. It applies to images, data and data takes, also inside
    /// substates `f` creates; other states are created as usual. The client
    /// behaves the same either way. The derive spells it
    /// `#[state(low_priority)]`.
    fn low_priority<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R
    where
        Self: Sized,
    {
        f(self)
    }

    /// Creates the values, signals and one-shot values that `f` makes in
    /// queue mode.
    ///
    /// Server callbacks then receive every change a client makes with
    /// `set_signal`, and every event of a signal, in arrival order. By
    /// default they receive only the latest change made while they are busy.
    /// A [`ValueTake`] keeps up to [`VALUE_TAKE_QUEUE`] untaken values in
    /// arrival order instead of only the latest. It applies to the states
    /// `f` creates itself, not to those of substates. The derive spells it
    /// `#[state(queue)]`.
    ///
    /// [`VALUE_TAKE_QUEUE`]: crate::VALUE_TAKE_QUEUE
    fn queue<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R
    where
        Self: Sized,
    {
        f(self)
    }

    /// Creates the one-shot values that `f` makes with a queue of up to
    /// `capacity` untaken values, at least one, as
    /// [`queue`](Self::queue) does with its default capacity. Other states
    /// are not affected. The derive spells it `#[state(queue = N)]`.
    fn queue_capacity<R>(&mut self, capacity: usize, f: impl FnOnce(&mut Self) -> R) -> R
    where
        Self: Sized,
    {
        let _ = capacity;
        f(self)
    }
}

/// A client state the receive task updates.
#[derive(Clone)]
pub(crate) enum ClientState {
    Value(Arc<dyn UpdateValue>),
    ValueTake(Arc<dyn UpdateValueTake>),
    Static(Arc<dyn UpdateValue>),
    Data(Arc<dyn UpdateData>),
    DataTake(Arc<dyn UpdateDataTake>),
    DataMulti(Arc<dyn UpdateMultiData>),
    DataMultiTake(Arc<dyn UpdateDataMultiTake>),
    Image(Image),
    ImageMulti(ImageMulti),
    Map(Arc<dyn UpdateMap>),
    Vec(Arc<dyn UpdateList>),
}

/// Typed lookups; a state of another kind is reported like a missing one.
macro_rules! lookups {
    ($($name:ident: $variant:ident => $ty:ty),* $(,)?) => {
        $(
            pub(crate) fn $name(&self, id: u64) -> Option<&$ty> {
                match self.states.get(&id) {
                    Some(ClientState::$variant(state)) => Some(state),
                    _ => None,
                }
            }
        )*
    };
}

/// Every client state, by id.
///
/// A hand-written `State` that registers one path under two kinds keeps only
/// the last; a generated server refuses such a layout anyway.
#[derive(Clone)]
pub(crate) struct ValuesList {
    pub(crate) states: NoHashMap<u64, ClientState>,
    /// The engine of the file transfer endpoints.
    pub(crate) transfers: Engine,
}

impl ValuesList {
    fn new(transfers: Engine) -> Self {
        Self {
            states: NoHashMap::default(),
            transfers,
        }
    }

    lookups! {
        value: Value => Arc<dyn UpdateValue>,
        value_take: ValueTake => Arc<dyn UpdateValueTake>,
        static_value: Static => Arc<dyn UpdateValue>,
        data: Data => Arc<dyn UpdateData>,
        data_take: DataTake => Arc<dyn UpdateDataTake>,
        data_multi: DataMulti => Arc<dyn UpdateMultiData>,
        data_multi_take: DataMultiTake => Arc<dyn UpdateDataMultiTake>,
        image: Image => Image,
        image_multi: ImageMulti => ImageMulti,
        map: Map => Arc<dyn UpdateMap>,
        vec: Vec => Arc<dyn UpdateList>,
    }

    /// Drops acknowledgement duties and partial transfers left over from a
    /// closed connection.
    pub(crate) fn disconnected(&self) {
        for state in self.states.values() {
            match state {
                ClientState::ValueTake(state) => state.disconnected(),
                ClientState::Data(state) => state.disconnected(),
                ClientState::DataMulti(state) => state.disconnected(),
                ClientState::DataTake(state) => state.disconnected(),
                ClientState::DataMultiTake(state) => state.disconnected(),
                ClientState::Image(state) => state.disconnected(),
                ClientState::ImageMulti(state) => state.disconnected(),
                ClientState::Map(state) => state.disconnected(),
                ClientState::Vec(state) => state.disconnected(),
                ClientState::Value(_) | ClientState::Static(_) => {}
            }
        }
    }

    /// Drops the take values a closed connection left untaken, once a new
    /// connection is accepted and before any of its messages is handled.
    pub(crate) fn connected(&self) {
        for state in self.states.values() {
            match state {
                ClientState::ValueTake(state) => state.connected(),
                ClientState::DataTake(state) => state.connected(),
                ClientState::DataMultiTake(state) => state.connected(),
                _ => {}
            }
        }
    }
}

/// The full path and id of the state `name` below `parent`.
pub(crate) fn state_path(parent: &str, name: &str) -> (String, u64) {
    let path = format!("{parent}.{name}");
    let id = generate_value_id(&path);
    (path, id)
}

// Stable tags mixed into the layout hash to distinguish state kinds. These
// values are part of the client/server compatibility contract; do not reorder
// or reuse them.
pub(crate) const VALUE_HASH_ID: u8 = 0;
pub(crate) const VALUE_TAKE_HASH_ID: u8 = 1;
pub(crate) const ATOMIC_HASH_ID: u8 = 2;
pub(crate) const STATIC_HASH_ID: u8 = 3;
pub(crate) const STATIC_ATOMIC_HASH_ID: u8 = 4;
pub(crate) const SIGNAL_HASH_ID: u8 = 5;
pub(crate) const MAP_HASH_ID: u8 = 6;
pub(crate) const VEC_HASH_ID: u8 = 7;
pub(crate) const DATA_HASH_ID: u8 = 8;
pub(crate) const DATA_MULTI_HASH_ID: u8 = 9;
pub(crate) const DATA_TAKE_HASH_ID: u8 = 10;
pub(crate) const DATA_MULTI_TAKE_HASH_ID: u8 = 11;
pub(crate) const IMAGE_MULTI_HASH_ID: u8 = 12;
pub(crate) const DOWNLOAD_HASH_ID: u8 = 13;
pub(crate) const UPLOAD_HASH_ID: u8 = 14;

/// Runtime state factory used internally by [`crate::ClientBuilder`].
pub struct StatesCreatorClient {
    val: ValuesList,
    sender: MessageSender,
    context: egui::Context,
    parent: String,
    version_hasher: StableHasher,
    /// The queue capacity of the one-shot values created now, `None` for
    /// none.
    take_queue: Option<usize>,
}

impl StatesCreatorClient {
    pub(crate) fn new(sender: MessageSender, context: egui::Context, parent: String) -> Self {
        let repaint_context = context.clone();
        let repaint: Repaint = Arc::new(move |delay| match delay {
            Some(delay) => repaint_context.request_repaint_after(delay),
            None => repaint_context.request_repaint(),
        });
        let transfers = Engine::new(Side::Client, Some(repaint));
        Self::with_transfers(sender, context, parent, transfers)
    }

    fn with_transfers(
        sender: MessageSender,
        context: egui::Context,
        parent: String,
        transfers: Engine,
    ) -> Self {
        Self {
            val: ValuesList::new(transfers),
            sender,
            context,
            parent,
            version_hasher: StableHasher::new(),
            take_queue: None,
        }
    }

    /// Returns the stable layout hash accumulated while constructing states.
    pub fn get_version_hash(&self) -> u64 {
        self.version_hasher.finish()
    }

    pub(crate) fn get_values(self) -> ValuesList {
        let mut val = self.val;
        val.states.shrink_to_fit();
        val
    }
}

impl StatesCreator for StatesCreatorClient {
    fn substate<S: State>(&mut self, name: &str) -> S {
        let parent = format!("{}.{}", self.parent, name);
        let mut creator = StatesCreatorClient::with_transfers(
            self.sender.clone(),
            self.context.clone(),
            parent,
            self.val.transfers.clone(),
        );
        let substate = S::new(&mut creator);
        creator
            .version_hasher
            .finish()
            .hash(&mut self.version_hasher);

        self.val.states.extend(creator.val.states);

        substate
    }

    fn value<T>(&mut self, name: &str, value: T) -> Value<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Send
            + Sync
            + Clone
            + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, VALUE_HASH_ID);

        let value = Value::new(name, id, type_id, value, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::Value(Arc::new(value.clone())));
        value
    }

    fn value_take<T>(&mut self, name: &str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Typed + Send + Sync + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, VALUE_TAKE_HASH_ID);

        let value = ValueTake::new(name, id, type_id, self.take_queue, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::ValueTake(Arc::new(value.clone())));
        value
    }

    fn atomic<T>(&mut self, name: &str, value: T) -> ValueAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Send
            + Sync
            + Clone
            + Atomic
            + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, ATOMIC_HASH_ID);

        let value = ValueAtomic::new(name, id, type_id, value, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::Value(Arc::new(value.clone())));
        value
    }

    fn add_static<T>(&mut self, name: &str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, STATIC_HASH_ID);

        let value = Static::new(name, type_id, value);

        self.val
            .states
            .insert(id, ClientState::Static(Arc::new(value.clone())));
        value
    }

    fn static_atomic<T>(&mut self, name: &'static str, value: T) -> StaticAtomic<T>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Clone
            + Send
            + Sync
            + Atomic
            + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, STATIC_ATOMIC_HASH_ID);

        let value = StaticAtomic::new(name, type_id, value);

        self.val
            .states
            .insert(id, ClientState::Static(Arc::new(value.clone())));
        value
    }

    fn image(&mut self, name: &str) -> Image {
        let (name, id) = state_path(&self.parent, name);
        hash_id(&mut self.version_hasher, id);

        let value = Image::new(name, id, self.sender.clone(), self.context.clone());

        self.val
            .states
            .insert(id, ClientState::Image(value.clone()));
        value
    }

    fn image_multi(&mut self, name: &str) -> ImageMulti {
        let (name, id) = state_path(&self.parent, name);
        hash_id_kind(&mut self.version_hasher, id, IMAGE_MULTI_HASH_ID);

        let value = ImageMulti::new(name, id, self.sender.clone(), self.context.clone());
        self.val
            .states
            .insert(id, ClientState::ImageMulti(value.clone()));
        value
    }

    fn signal<T>(&mut self, name: &str) -> Signal<T>
    where
        T: Serialize + Typed + Clone + Send + Sync + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, SIGNAL_HASH_ID);

        Signal::new(name, id, type_id, self.sender.clone())
    }

    fn map<K, V>(&mut self, name: &str) -> MapState<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Send + Sync + Typed + 'static,
        V: Clone + for<'a> Deserialize<'a> + Send + Sync + Typed + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = ObjectType::map_hash(&K::get_type(), &V::get_type());
        hash_id_type(&mut self.version_hasher, id, type_id, MAP_HASH_ID);

        let value = MapState::new(name, type_id);
        self.val
            .states
            .insert(id, ClientState::Map(Arc::new(value.clone())));
        value
    }

    fn vec<T>(&mut self, name: &str) -> VecState<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Send + Sync + Typed + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, VEC_HASH_ID);

        let value = VecState::new(name, type_id);

        self.val
            .states
            .insert(id, ClientState::Vec(Arc::new(value.clone())));
        value
    }

    fn data<T, H>(&mut self, name: &str) -> Data<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            DATA_HASH_ID,
        );

        let data = Data::new(name, id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::Data(Arc::new(data.clone())));
        data
    }

    fn data_multi<T, H>(&mut self, name: &'static str) -> DataMulti<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            DATA_MULTI_HASH_ID,
        );

        let multi_data = DataMulti::new(name, id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::DataMulti(Arc::new(multi_data.clone())));
        multi_data
    }

    fn data_take<T, H>(&mut self, name: &'static str) -> DataTake<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            DATA_TAKE_HASH_ID,
        );

        let data_take = DataTake::new(name, id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::DataTake(Arc::new(data_take.clone())));
        data_take
    }

    fn data_multi_take<T, H>(&mut self, name: &'static str) -> DataMultiTake<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            DATA_MULTI_TAKE_HASH_ID,
        );

        let data_multi_take = DataMultiTake::new(name, id, self.sender.clone());

        self.val.states.insert(
            id,
            ClientState::DataMultiTake(Arc::new(data_multi_take.clone())),
        );
        data_multi_take
    }

    fn download<M: TransferMeta>(&mut self, name: &str) -> Download<M> {
        let (_, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            meta_type_id::<M>(),
            DOWNLOAD_HASH_ID,
        );
        self.val
            .transfers
            .register(id, Role::Receive, meta_type_id::<M>());
        Download::new(id, self.val.transfers.clone())
    }

    fn queue<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.queue_capacity(crate::VALUE_TAKE_QUEUE, f)
    }

    fn queue_capacity<R>(&mut self, capacity: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        let outer = self.take_queue.replace(capacity);
        let result = f(self);
        self.take_queue = outer;
        result
    }

    fn upload<M: TransferMeta>(&mut self, name: &str) -> Upload<M> {
        let (_, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            meta_type_id::<M>(),
            UPLOAD_HASH_ID,
        );
        self.val
            .transfers
            .register(id, Role::Send, meta_type_id::<M>());
        Upload::new(id, self.val.transfers.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::states::values::UpdateValueTake;

    /// Fills `value` with `count` values and tells whether the last one
    /// overflowed the queue.
    fn overflows(value: &ValueTake<u8>, count: usize) -> bool {
        let mut overflowed = false;
        for _ in 0..count {
            let data = postcard::to_stdvec(&1_u8).unwrap();
            overflowed = value
                .update_take(<u8 as crate::Typed>::get_type().get_hash(), &data, false)
                .is_err();
        }
        overflowed
    }

    #[test]
    fn queue_scopes_set_the_capacity_of_value_takes() {
        let (sender, _receiver) = MessageSender::new();
        let mut creator = StatesCreatorClient::new(sender, egui::Context::default(), "root".into());
        let plain = creator.value_take::<u8>("plain");
        let queued = creator.queue(|c| c.value_take::<u8>("queued"));
        let few = creator.queue_capacity(2, |c| c.value_take::<u8>("few"));
        let after = creator.value_take::<u8>("after");

        assert!(!overflows(&plain, 3));
        assert_eq!(plain.len(), 1);
        assert!(!overflows(&queued, crate::VALUE_TAKE_QUEUE));
        assert!(overflows(&queued, 1));
        assert!(!overflows(&few, 2));
        assert!(overflows(&few, 1));
        assert!(!overflows(&after, 3), "the scope leaked");
    }
}
