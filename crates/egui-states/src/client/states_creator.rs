//! State-handle factories and stable state-layout hashing.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::messages::MessageSender;
use crate::client::states::atomics::{Atomic, AtomicStatic};
use crate::client::states::data::{
    Data, DataMulti, UpdateData, UpdateMultiData, private::GetDataType,
};
use crate::client::states::data_take::{
    DataMultiTake, DataTake, UpdateDataMultiTake, UpdateDataTake,
};
use crate::client::states::image::{Image, ImageMulti};
use crate::client::states::initial_value::InitialValue;
use crate::client::states::map::{MapState, UpdateMap};
use crate::client::states::values::{
    GetQueueType, Signal, Static, StaticAtomic, UpdateValue, UpdateValueTake, Value, ValueAtomic,
    ValueTake,
};
use crate::client::states::vec::{UpdateList, VecState};
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
    fn value<T, Q>(&mut self, name: &'static str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Send
            + Sync
            + Clone
            + 'static,
        Q: GetQueueType;

    /// Creates a one-shot value received by and consumed on the client.
    fn value_take<T>(&mut self, name: &'static str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Typed + Send + Sync + 'static;

    /// Creates an atomic synchronized read/write value.
    fn atomic<T, Q>(&mut self, name: &'static str, value: T) -> ValueAtomic<T, Q>
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
        Q: GetQueueType;

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
            + AtomicStatic
            + 'static;

    /// Creates a client-to-server event carrying values of type `T`.
    fn signal<T, Q>(&mut self, name: &'static str) -> Signal<T, Q>
    where
        T: Serialize + Typed + Clone + Send + Sync + 'static,
        Q: GetQueueType;

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
    /// Creates a contiguous server-controlled numeric buffer.
    fn data<T>(&mut self, name: &'static str) -> Data<T>
    where
        T: GetDataType + Send + Sync + 'static;

    #[allow(private_bounds)]
    /// Creates keyed server-controlled numeric buffers.
    fn data_multi<T>(&mut self, name: &'static str) -> DataMulti<T>
    where
        T: GetDataType + Send + Sync + 'static;

    #[allow(private_bounds)]
    /// Creates a one-shot numeric buffer consumed by the client.
    fn data_take<T>(&mut self, name: &'static str) -> DataTake<T>
    where
        T: GetDataType + Send + Sync + 'static;

    #[allow(private_bounds)]
    /// Creates keyed one-shot numeric buffers consumed by the client.
    fn data_multi_take<T>(&mut self, name: &'static str) -> DataMultiTake<T>
    where
        T: GetDataType + Send + Sync + 'static;
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
}

impl ValuesList {
    fn new() -> Self {
        Self {
            states: NoHashMap::default(),
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

/// Runtime state factory used internally by [`crate::ClientBuilder`].
pub struct StatesCreatorClient {
    val: ValuesList,
    sender: MessageSender,
    context: egui::Context,
    parent: String,
    version_hasher: StableHasher,
}

impl StatesCreatorClient {
    pub(crate) fn new(sender: MessageSender, context: egui::Context, parent: String) -> Self {
        Self {
            val: ValuesList::new(),
            sender,
            context,
            parent,
            version_hasher: StableHasher::new(),
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
        let mut creator =
            StatesCreatorClient::new(self.sender.clone(), self.context.clone(), parent);
        let substate = S::new(&mut creator);
        creator
            .version_hasher
            .finish()
            .hash(&mut self.version_hasher);

        self.val.states.extend(creator.val.states);

        substate
    }

    fn value<T, Q>(&mut self, name: &str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a>
            + Serialize
            + Typed
            + InitialValue
            + Send
            + Sync
            + Clone
            + 'static,
        Q: GetQueueType,
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

        let value = ValueTake::new(name, id, type_id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::ValueTake(Arc::new(value.clone())));
        value
    }

    fn atomic<T, Q>(&mut self, name: &str, value: T) -> ValueAtomic<T, Q>
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
        Q: GetQueueType,
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

        let value = Static::new(name, id, type_id, value, self.sender.clone());

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
            + AtomicStatic
            + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(&mut self.version_hasher, id, type_id, STATIC_ATOMIC_HASH_ID);

        let value = StaticAtomic::new(name, id, type_id, value, self.sender.clone());

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

    fn signal<T, Q>(&mut self, name: &str) -> Signal<T, Q>
    where
        T: Serialize + Typed + Clone + Send + Sync + 'static,
        Q: GetQueueType,
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

    fn data<T>(&mut self, name: &str) -> Data<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(&mut self.version_hasher, id, T::get_type_id(), DATA_HASH_ID);

        let data = Data::new(name, id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::Data(Arc::new(data.clone())));
        data
    }

    fn data_multi<T>(&mut self, name: &'static str) -> DataMulti<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            DATA_MULTI_HASH_ID,
        );

        let multi_data = DataMulti::new(name, id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::DataMulti(Arc::new(multi_data.clone())));
        multi_data
    }

    fn data_take<T>(&mut self, name: &'static str) -> DataTake<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            DATA_TAKE_HASH_ID,
        );

        let data_take = DataTake::new(name, id, self.sender.clone());

        self.val
            .states
            .insert(id, ClientState::DataTake(Arc::new(data_take.clone())));
        data_take
    }

    fn data_multi_take<T>(&mut self, name: &'static str) -> DataMultiTake<T>
    where
        T: GetDataType + Send + Sync + 'static,
    {
        let (name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            T::get_type_id(),
            DATA_MULTI_TAKE_HASH_ID,
        );

        let data_multi_take = DataMultiTake::new(name, id, self.sender.clone());

        self.val.states.insert(
            id,
            ClientState::DataMultiTake(Arc::new(data_multi_take.clone())),
        );
        data_multi_take
    }
}
