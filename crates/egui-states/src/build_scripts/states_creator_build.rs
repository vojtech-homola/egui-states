use std::collections::{BTreeMap, btree_map::Entry};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::State;
use crate::client::states::atomics::{Atomic, AtomicStatic};
use crate::client::states::data::{
    Data, DataMulti, HeaderValue, data_type_hash, private::GetDataType,
};
use crate::client::states::data_take::{DataMultiTake, DataTake};
use crate::client::states::file_transfer::{Download, Upload};
use crate::client::states::image::{Image, ImageMulti};
use crate::client::states::initial_value::{InitValue, InitialValue};

use crate::client::messages::MessageSender;
use crate::client::states::map::MapState;
use crate::client::states::values::{
    GetQueueType, Signal, Static, StaticAtomic, Value, ValueAtomic, ValueTake,
};
use crate::client::states::vec::VecState;
use crate::client::states_creator::{
    self, StatesCreator, hash_id, hash_id_kind, hash_id_type, state_path,
};
use crate::file_transfer::engine::{Engine, Side};
use crate::file_transfer::{TransferMeta, meta_type_id};
use crate::hashing::StableHasher;
use crate::protocol::data::DataType;
use crate::typed::{ObjectType, RustDerive, Typed};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RustDerives {
    enums: BTreeMap<String, Vec<String>>,
    structs: BTreeMap<String, Vec<String>>,
    conflicts: Vec<String>,
}

impl RustDerives {
    fn insert(&mut self, derive: RustDerive) {
        match derive {
            RustDerive::Enum(name, derives) => {
                Self::insert_type(&mut self.enums, &mut self.conflicts, "enum", name, derives)
            }
            RustDerive::Struct(name, derives) => Self::insert_type(
                &mut self.structs,
                &mut self.conflicts,
                "struct",
                name,
                derives,
            ),
        }
    }

    fn insert_type(
        types: &mut BTreeMap<String, Vec<String>>,
        conflicts: &mut Vec<String>,
        kind: &str,
        name: &str,
        derives: &[&str],
    ) {
        let derives = derives
            .iter()
            .map(|derive| (*derive).to_owned())
            .collect::<Vec<_>>();
        match types.entry(name.to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(derives);
            }
            Entry::Occupied(entry) if entry.get() != &derives => {
                conflicts.push(format!(
                    "{kind} {name} declared with inconsistent Rust derives: {:?} vs {:?}",
                    entry.get(),
                    derives
                ));
            }
            Entry::Occupied(_) => {}
        }
    }

    fn merge(&mut self, other: Self) {
        let Self {
            enums,
            structs,
            mut conflicts,
        } = other;
        self.conflicts.append(&mut conflicts);
        Self::merge_types(&mut self.enums, &mut self.conflicts, "enum", enums);
        Self::merge_types(&mut self.structs, &mut self.conflicts, "struct", structs);
    }

    fn merge_types(
        types: &mut BTreeMap<String, Vec<String>>,
        conflicts: &mut Vec<String>,
        kind: &str,
        other: BTreeMap<String, Vec<String>>,
    ) {
        for (name, derives) in other {
            match types.entry(name.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(derives);
                }
                Entry::Occupied(entry) if entry.get() != &derives => {
                    conflicts.push(format!(
                        "{kind} {name} declared with inconsistent Rust derives: {:?} vs {:?}",
                        entry.get(),
                        derives
                    ));
                }
                Entry::Occupied(_) => {}
            }
        }
    }

    pub(crate) fn validate(&self) {
        if let Some(conflict) = self.conflicts.first() {
            panic!("{conflict}");
        }
    }

    pub(crate) fn enum_derives(&self, name: &str) -> Option<&[String]> {
        self.enums.get(name).map(Vec::as_slice)
    }

    pub(crate) fn struct_derives(&self, name: &str) -> Option<&[String]> {
        self.structs.get(name).map(Vec::as_slice)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StateType {
    Value(String, ObjectType, InitValue, bool),
    ValueTake(String, ObjectType),
    Static(String, ObjectType, InitValue),
    Image(String),
    ImageMulti(String),
    ValueMap(String, ObjectType, ObjectType),
    ValueVec(String, ObjectType),
    Signal(String, ObjectType, bool),
    /// Name, element type and header type (`None` for a state without one).
    Data(String, DataType, Option<ObjectType>),
    DataTake(String, DataType, Option<ObjectType>),
    DataMulti(String, DataType, Option<ObjectType>),
    DataMultiTake(String, DataType, Option<ObjectType>),
    /// Name and metadata type (`None` for an endpoint without metadata).
    Download(String, Option<ObjectType>),
    Upload(String, Option<ObjectType>),
    SubState(String, &'static str, Vec<StateType>),
}

impl StateType {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Value(name, ..)
            | Self::ValueTake(name, ..)
            | Self::Static(name, ..)
            | Self::Image(name)
            | Self::ImageMulti(name)
            | Self::ValueMap(name, ..)
            | Self::ValueVec(name, ..)
            | Self::Signal(name, ..)
            | Self::Data(name, ..)
            | Self::DataTake(name, ..)
            | Self::DataMulti(name, ..)
            | Self::DataMultiTake(name, ..)
            | Self::Download(name, ..)
            | Self::Upload(name, ..)
            | Self::SubState(name, ..) => name,
        }
    }
}

pub(crate) struct StatesCreatorBuild {
    states: Vec<StateType>,
    rust_derives: RustDerives,
    parent: String,
    sender: MessageSender,
    version_hasher: StableHasher,
}

impl StatesCreatorBuild {
    pub fn new(parent: &str) -> Self {
        let (sender, _) = MessageSender::new();

        Self {
            states: Vec::new(),
            rust_derives: RustDerives::default(),
            parent: parent.to_string(),
            sender,
            version_hasher: StableHasher::new(),
        }
    }

    pub fn get_version_hash(&mut self) -> u64 {
        self.version_hasher.finish()
    }

    pub fn into_parts(self) -> (Vec<StateType>, RustDerives) {
        (self.states, self.rust_derives)
    }

    fn collect_rust_derives<T: Typed>(&mut self) {
        for derive in T::rust_derives() {
            self.rust_derives.insert(derive);
        }
    }

    /// The metadata type of a transfer endpoint, collecting its Rust derives.
    fn meta_type<M: TransferMeta>(&mut self) -> Option<ObjectType> {
        match M::get_type() {
            ObjectType::Empty => None,
            object_type => {
                self.collect_rust_derives::<M>();
                Some(object_type)
            }
        }
    }

    /// The header type of a Data state, collecting its Rust derives.
    fn header_type<H: HeaderValue>(&mut self) -> Option<ObjectType> {
        match H::get_type() {
            ObjectType::Empty => None,
            object_type => {
                self.collect_rust_derives::<H>();
                Some(object_type)
            }
        }
    }
}

impl StatesCreator for StatesCreatorBuild {
    fn substate<S: State>(&mut self, name: &str) -> S {
        let parent = format!("{}.{}", self.parent, name);

        let mut builder = Self::new(&parent);
        let substate = S::new(&mut builder);
        builder
            .version_hasher
            .finish()
            .hash(&mut self.version_hasher);

        let (states, rust_derives) = builder.into_parts();
        self.rust_derives.merge(rust_derives);
        self.states
            .push(StateType::SubState(name.to_owned(), S::NAME, states));

        substate
    }

    fn value<T, Q>(&mut self, name: &str, value: T) -> Value<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Typed + InitialValue,
        Q: GetQueueType,
    {
        self.collect_rust_derives::<T>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VALUE_HASH_ID,
        );

        let init = value.init_value();
        let value = Value::new(full_name, id, type_id, value, self.sender.clone());

        self.states.push(StateType::Value(
            name.to_owned(),
            T::get_type(),
            init,
            Q::is_queue(),
        ));

        value
    }

    fn value_take<T>(&mut self, name: &str) -> ValueTake<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Typed + Send + Sync + 'static,
    {
        self.collect_rust_derives::<T>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VALUE_TAKE_HASH_ID,
        );

        let value = ValueTake::new(full_name, id, type_id, self.sender.clone());

        self.states
            .push(StateType::ValueTake(name.to_owned(), T::get_type()));

        value
    }

    fn atomic<T, Q>(&mut self, name: &str, value: T) -> ValueAtomic<T, Q>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Typed + InitialValue + Atomic,
        Q: GetQueueType,
    {
        self.collect_rust_derives::<T>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::ATOMIC_HASH_ID,
        );

        let init = value.init_value();
        let value = ValueAtomic::new(full_name, id, type_id, value, self.sender.clone());

        self.states.push(StateType::Value(
            name.to_owned(),
            T::get_type(),
            init,
            Q::is_queue(),
        ));

        value
    }

    fn add_static<T>(&mut self, name: &str, value: T) -> Static<T>
    where
        T: for<'a> Deserialize<'a> + Serialize + Clone + Typed + InitialValue,
    {
        self.collect_rust_derives::<T>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::STATIC_HASH_ID,
        );

        let init = value.init_value();
        let value = Static::new(full_name, type_id, value);

        self.states
            .push(StateType::Static(name.to_owned(), T::get_type(), init));
        value
    }

    fn static_atomic<T>(&mut self, name: &str, value: T) -> StaticAtomic<T>
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
        self.collect_rust_derives::<T>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::STATIC_ATOMIC_HASH_ID,
        );

        let init = value.init_value();
        let value = StaticAtomic::new(full_name, type_id, value);

        self.states
            .push(StateType::Static(name.to_owned(), T::get_type(), init));
        value
    }

    fn image(&mut self, name: &str) -> Image {
        let (full_name, id) = state_path(&self.parent, name);
        hash_id(&mut self.version_hasher, id);

        // Codegen only walks the layout, so the handle needs no real context.
        let value = Image::new(full_name, id, self.sender.clone(), egui::Context::default());

        self.states.push(StateType::Image(name.to_owned()));

        value
    }

    fn image_multi(&mut self, name: &str) -> ImageMulti {
        let (full_name, id) = state_path(&self.parent, name);
        hash_id_kind(
            &mut self.version_hasher,
            id,
            states_creator::IMAGE_MULTI_HASH_ID,
        );

        let value = ImageMulti::new(full_name, id, self.sender.clone(), egui::Context::default());
        self.states.push(StateType::ImageMulti(name.to_owned()));
        value
    }

    fn signal<T, Q>(&mut self, name: &str) -> Signal<T, Q>
    where
        T: Serialize + Clone + Typed,
        Q: GetQueueType,
    {
        self.collect_rust_derives::<T>();
        let (_, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::SIGNAL_HASH_ID,
        );

        let signal = Signal::new(name.to_owned(), id, type_id, self.sender.clone());

        self.states.push(StateType::Signal(
            name.to_owned(),
            T::get_type(),
            Q::is_queue(),
        ));

        signal
    }

    fn map<K, V>(&mut self, name: &str) -> MapState<K, V>
    where
        K: Hash + Eq + Clone + for<'a> Deserialize<'a> + Typed,
        V: Clone + for<'a> Deserialize<'a> + Typed,
    {
        self.collect_rust_derives::<K>();
        self.collect_rust_derives::<V>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = ObjectType::map_hash(&K::get_type(), &V::get_type());
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::MAP_HASH_ID,
        );

        let value = MapState::new(full_name, type_id);

        self.states.push(StateType::ValueMap(
            name.to_owned(),
            K::get_type(),
            V::get_type(),
        ));
        value
    }

    fn vec<T>(&mut self, name: &str) -> VecState<T>
    where
        T: Clone + for<'a> Deserialize<'a> + Typed,
    {
        self.collect_rust_derives::<T>();
        let (full_name, id) = state_path(&self.parent, name);
        let type_id = T::get_type().get_hash();
        hash_id_type(
            &mut self.version_hasher,
            id,
            type_id,
            states_creator::VEC_HASH_ID,
        );

        let value = VecState::new(full_name, type_id);

        self.states
            .push(StateType::ValueVec(name.to_owned(), T::get_type()));

        value
    }

    fn data<T, H>(&mut self, name: &str) -> Data<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (full_name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            states_creator::DATA_HASH_ID,
        );

        let value = Data::new(full_name, id, self.sender.clone());

        let header = self.header_type::<H>();
        self.states
            .push(StateType::Data(name.to_owned(), T::get_type(), header));
        value
    }

    fn data_multi<T, H>(&mut self, name: &str) -> DataMulti<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (full_name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            states_creator::DATA_MULTI_HASH_ID,
        );

        let value = DataMulti::new(full_name, id, self.sender.clone());

        let header = self.header_type::<H>();
        self.states
            .push(StateType::DataMulti(name.to_owned(), T::get_type(), header));
        value
    }

    fn data_take<T, H>(&mut self, name: &str) -> DataTake<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (full_name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            states_creator::DATA_TAKE_HASH_ID,
        );

        let value = DataTake::new(full_name, id, self.sender.clone());

        let header = self.header_type::<H>();
        self.states
            .push(StateType::DataTake(name.to_owned(), T::get_type(), header));
        value
    }

    fn data_multi_take<T, H>(&mut self, name: &str) -> DataMultiTake<T, H>
    where
        T: GetDataType + Send + Sync + 'static,
        H: HeaderValue,
    {
        let (full_name, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            data_type_hash::<T, H>(),
            states_creator::DATA_MULTI_TAKE_HASH_ID,
        );

        let value = DataMultiTake::new(full_name, id, self.sender.clone());

        let header = self.header_type::<H>();
        self.states.push(StateType::DataMultiTake(
            name.to_owned(),
            T::get_type(),
            header,
        ));
        value
    }

    fn download<M: TransferMeta>(&mut self, name: &str) -> Download<M> {
        let (_, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            meta_type_id::<M>(),
            states_creator::DOWNLOAD_HASH_ID,
        );
        let meta = self.meta_type::<M>();
        self.states.push(StateType::Download(name.to_owned(), meta));
        // The handle is discarded; no connection ever runs.
        Download::new(id, Engine::new(Side::Client, None))
    }

    fn upload<M: TransferMeta>(&mut self, name: &str) -> Upload<M> {
        let (_, id) = state_path(&self.parent, name);
        hash_id_type(
            &mut self.version_hasher,
            id,
            meta_type_id::<M>(),
            states_creator::UPLOAD_HASH_ID,
        );
        let meta = self.meta_type::<M>();
        self.states.push(StateType::Upload(name.to_owned(), meta));
        Upload::new(id, Engine::new(Side::Client, None))
    }
}
