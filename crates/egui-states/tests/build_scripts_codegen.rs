#![cfg(feature = "build_scripts")]

use std::collections::HashMap;
use std::path::PathBuf;

use egui_states::build_scripts::{generate_python, generate_rust};
use egui_states::{
    ClientBuilder, Data, DataMulti, DataMultiTake, DataTake, Download, Image, ImageMulti, MapState,
    Signal, State, StatesCreator, Static, StaticAtomic, Upload, Value, ValueAtomic, ValueTake,
    VecState,
};

#[egui_states::typed(rust_derive(Debug, PartialEq, Eq, Hash))]
#[derive(Clone, Default, PartialEq, Eq, Hash, egui_states::InitialValue)]
struct GeneratedInner {
    enabled: bool,
}

#[egui_states::typed(rust_derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    egui_states::serde::Serialize,
    egui_states::Typed
))]
#[derive(Clone, Default, PartialEq, Eq, Hash, egui_states::InitialValue)]
struct GeneratedOuter {
    inner: Option<GeneratedInner>,
}

#[egui_states::typed(rust_derive(Debug, PartialOrd, Ord))]
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, egui_states::InitialValue)]
enum GeneratedEnum {
    #[default]
    First,
    Second,
}

mod first_collision {
    #[egui_states::typed(rust_derive(Debug))]
    #[derive(Clone, Default, egui_states::InitialValue)]
    pub(super) struct Collision {
        pub(super) value: bool,
    }
}

mod second_collision {
    #[egui_states::typed(rust_derive(Hash))]
    #[derive(Clone, Default, egui_states::InitialValue)]
    pub(super) struct Collision {
        pub(super) value: bool,
    }
}

/// A state class holding map-valued states, used twice by [`Root`].
struct Leaf;

impl State for Leaf {
    const NAME: &'static str = "Leaf";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<HashMap<u16, u32>> = c.value(
            "numbers",
            HashMap::from([(10, 10), (1, 1), (20, 20), (2, 2), (3, 3)]),
        );
        let _: Value<HashMap<String, u8>> = c.value(
            "labels",
            HashMap::from([
                ("b".to_string(), 1),
                ("a".to_string(), 2),
                ("c".to_string(), 3),
            ]),
        );
        Self
    }
}

struct Root;

impl State for Root {
    const NAME: &'static str = "Root";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Leaf = c.substate("first");
        let _: Leaf = c.substate("second");
        let _: Image = c.image("image");
        let _: ImageMulti = c.image_multi("images");
        let _: Value<Option<GeneratedOuter>> = c.value(
            "custom",
            Some(GeneratedOuter {
                inner: Some(GeneratedInner { enabled: true }),
            }),
        );
        let _: Value<GeneratedEnum> = c.value("custom_enum", GeneratedEnum::First);
        Self
    }
}

struct ImageRoot;

impl State for ImageRoot {
    const NAME: &'static str = "KindRoot";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Image = c.image("value");
        Self
    }
}

struct ImageMultiRoot;

impl State for ImageMultiRoot {
    const NAME: &'static str = "KindRoot";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: ImageMulti = c.image_multi("value");
        Self
    }
}

fn output_dir(name: &str, run: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "egui_states_codegen_{}_{name}_{run}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn generate(dir: &PathBuf) -> (String, String) {
    generate_rust::<Root>(dir).unwrap();
    generate_python::<Root>(dir.join("python")).unwrap();
    (
        std::fs::read_to_string(dir.join("mod.rs")).unwrap(),
        std::fs::read_to_string(dir.join("python/__init__.py")).unwrap(),
    )
}

/// Generated bindings must be byte-identical between runs so that repeated
/// builds do not rewrite the files.
#[test]
fn generated_bindings_are_stable_across_runs() {
    let mut expected: Option<Vec<Vec<u8>>> = None;

    for run in 0..16 {
        let dir = output_dir("stable", run);
        generate(&dir);
        let generated = all_files(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        match &expected {
            None => expected = Some(generated),
            Some(expected) => assert_eq!(
                *expected, generated,
                "generated bindings changed between runs"
            ),
        }
    }
}

/// Both generators emit map entries ordered by key.
#[test]
fn map_initial_values_are_ordered_by_key() {
    let dir = output_dir("order", 0);
    let (rust, python) = generate(&dir);

    assert!(
        rust.contains("[(1u16, 1u32), (2u16, 2u32), (3u16, 3u32), (10u16, 10u32), (20u16, 20u32)]"),
        "unexpected Rust map ordering:\n{rust}"
    );
    assert!(
        python.contains("{1: 1, 2: 2, 3: 3, 10: 10, 20: 20}"),
        "unexpected Python map ordering:\n{python}"
    );
    assert!(
        python.contains(r#"{"a": 2, "b": 1, "c": 3}"#),
        "unexpected Python string-key ordering:\n{python}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn generated_hash(output: &str, marker: &str) -> u64 {
    let start = output.find(marker).expect("generated version hash marker") + marker.len();
    output[start..]
        .split(|character: char| !character.is_ascii_digit())
        .next()
        .expect("generated version hash")
        .parse()
        .expect("numeric generated version hash")
}

#[test]
fn image_multi_codegen_and_layout_hash_are_distinct() {
    let image_dir = output_dir("image_kind", 0);
    let multi_dir = output_dir("image_multi_kind", 0);

    generate_rust::<ImageRoot>(&image_dir).unwrap();
    generate_python::<ImageRoot>(image_dir.join("python")).unwrap();
    generate_rust::<ImageMultiRoot>(&multi_dir).unwrap();
    generate_python::<ImageMultiRoot>(multi_dir.join("python")).unwrap();

    let image_rust = std::fs::read_to_string(image_dir.join("mod.rs")).unwrap();
    let image_python = std::fs::read_to_string(image_dir.join("python/__init__.py")).unwrap();
    let multi_rust = std::fs::read_to_string(multi_dir.join("mod.rs")).unwrap();
    let multi_python = std::fs::read_to_string(multi_dir.join("python/__init__.py")).unwrap();

    let image_hash = generated_hash(&image_rust, "pub const VERSION_HASH: u64 = ");
    let multi_hash = generated_hash(&multi_rust, "pub const VERSION_HASH: u64 = ");
    assert_eq!(
        image_hash,
        generated_hash(&image_python, "VERSION_HASH: int = ")
    );
    assert_eq!(
        multi_hash,
        generated_hash(&multi_python, "VERSION_HASH: int = ")
    );
    assert_ne!(image_hash, multi_hash);
    assert!(multi_rust.contains("pub value: s::ImageMulti"));
    assert!(multi_python.contains("self.value: s.ImageMulti = s.ImageMulti()"));

    let _ = std::fs::remove_dir_all(&image_dir);
    let _ = std::fs::remove_dir_all(&multi_dir);
}

struct ConflictingDerives;

impl State for ConflictingDerives {
    const NAME: &'static str = "ConflictingDerives";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<first_collision::Collision> = c.value("first", Default::default());
        let _: Value<second_collision::Collision> = c.value("second", Default::default());
        Self
    }
}

#[test]
#[should_panic(
    expected = "struct Collision declared with inconsistent Rust derives: [\"Debug\"] vs [\"Hash\"]"
)]
fn inconsistent_rust_derives_for_one_generated_name_are_rejected() {
    let dir = output_dir("conflicting_rust_derives", 0);
    let _ = generate_rust::<ConflictingDerives>(&dir);
}

#[test]
fn inconsistent_rust_derives_do_not_affect_python_generation() {
    let dir = output_dir("python_conflicting_rust_derives", 0);
    generate_python::<ConflictingDerives>(&dir).unwrap();

    assert!(dir.join("structs.py").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

fn all_files(dir: &std::path::Path) -> Vec<Vec<u8>> {
    [
        "mod.rs",
        "structs.rs",
        "enums.rs",
        "python/__init__.py",
        "python/structs.py",
        "python/enums.py",
    ]
    .into_iter()
    .map(|file| std::fs::read(dir.join(file)).unwrap())
    .collect()
}

#[test]
fn unchanged_generation_preserves_old_timestamps_for_all_six_files() {
    let dir = output_dir("mtime", 0);
    generate(&dir);
    let before = all_files(&dir);
    let paths: Vec<_> = [
        "mod.rs",
        "structs.rs",
        "enums.rs",
        "python/__init__.py",
        "python/structs.py",
        "python/enums.py",
    ]
    .into_iter()
    .map(|file| dir.join(file))
    .collect();
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(946684800);
    for path in &paths {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    let timestamps: Vec<_> = paths
        .iter()
        .map(|path| path.metadata().unwrap().modified().unwrap())
        .collect();
    generate(&dir);
    assert_eq!(all_files(&dir), before);
    for (path, timestamp) in paths.iter().zip(timestamps) {
        assert_eq!(
            path.metadata().unwrap().modified().unwrap(),
            timestamp,
            "rewrote unchanged {}",
            path.display()
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Uses every state kind, so the build-time and runtime layout hashes are
/// compared for all of them.
struct AllKinds;

impl State for AllKinds {
    const NAME: &'static str = "AllKinds";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("value", 3);
        let _: Value<String> = c.queue(|c| c.value("queued", "text".into()));
        let _: ValueAtomic<f64> = c.atomic("atomic", 0.5);
        let _: Static<GeneratedEnum> = c.add_static("fixed", GeneratedEnum::Second);
        let _: StaticAtomic<[f32; 2]> = c.static_atomic("pair", [1.0, 2.0]);
        let _: ValueTake<String> = c.value_take("take");
        let _: Signal<u8> = c.signal("signal");
        let _: Signal<()> = c.queue(|c| c.signal("empty"));
        let _: Image = c.image("image");
        let _: ImageMulti = c.image_multi("images");
        let _: MapState<u16, GeneratedOuter> = c.map("map");
        let _: VecState<i32> = c.vec("list");
        let _: Data<u8> = c.data("data");
        let _: DataMulti<f32> = c.data_multi("data_multi");
        let _: DataTake<u16> = c.data_take("data_take");
        let _: DataMultiTake<i64> = c.data_multi_take("data_multi_take");
        let _: Leaf = c.substate("leaf");
        Self
    }
}

#[test]
fn client_and_generated_layout_hashes_match_for_every_state_kind() {
    let dir = output_dir("all_kinds", 0);
    generate_rust::<AllKinds>(&dir).unwrap();
    generate_python::<AllKinds>(dir.join("python")).unwrap();
    let rust = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    let python = std::fs::read_to_string(dir.join("python/__init__.py")).unwrap();

    let client = ClientBuilder::<AllKinds>::new(egui::Context::default(), 0).get_version_hash();
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        client
    );
    assert_eq!(generated_hash(&python, "VERSION_HASH: int = "), client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A substate without images or buffers, which takes no priority.
#[derive(State)]
#[allow(dead_code)]
struct ModesLeaf {
    count: Value<i32>,
}

/// Data, a marked image and substates inside a substate, for [`Modes`].
#[derive(State)]
#[allow(dead_code)]
struct ModesInner {
    buffer: Data<u8>,
    #[state(low_priority)]
    preview: Image,
    leaf: ModesLeaf,
    count: Value<i32>,
}

/// Low send priority and queue mode on every kind of field that takes them.
#[derive(State)]
#[allow(dead_code)]
struct Modes {
    plain: Image,
    #[state(low_priority)]
    camera: Image,
    #[state(low_priority)]
    inner: ModesInner,
    /// The same group at normal priority.
    ordered: ModesInner,
    #[state(queue)]
    clicks: Signal<u32>,
    #[state(queue)]
    progress: Value<f32>,
    #[state(queue)]
    level: ValueAtomic<f64>,
}

/// [`Modes`] without any mode.
#[derive(State)]
#[allow(dead_code)]
struct NoModes {
    plain: Image,
    camera: Image,
    inner: ModesInner,
    ordered: ModesInner,
    clicks: Signal<u32>,
    progress: Value<f32>,
    level: ValueAtomic<f64>,
}

/// The modes reach both bindings, but not the layout hash: they change only
/// how the server sends and calls back.
#[test]
fn bindings_carry_send_priority_and_queue_mode() {
    let dir = output_dir("modes", 0);
    generate_rust::<Modes>(&dir).unwrap();
    generate_python::<Modes>(dir.join("python")).unwrap();
    let rust = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    for expected in [
        "s::Image::new(server, format!(\"{parent}.plain\"), false)?",
        "s::Image::new(server, format!(\"{parent}.camera\"), true)?",
        // One group type serves both priorities: it takes the priority of
        // the field it is created for, unless a state inside is marked.
        "pub fn new(server: &s::StateServer, parent: &str, low_priority: bool) -> s::Result<Self>",
        "inner: ModesInner::new(server, &format!(\"{parent}.inner\"), true)?",
        "ordered: ModesInner::new(server, &format!(\"{parent}.ordered\"), false)?",
        "s::Data::<u8>::new(server, format!(\"{parent}.buffer\"), low_priority)?",
        "s::Image::new(server, format!(\"{parent}.preview\"), true)?",
        "leaf: ModesLeaf::new(server, &format!(\"{parent}.leaf\"))?",
        "s::Value::new(server, format!(\"{parent}.count\"), 0i32, false)?",
        "s::Signal::<u32>::new(server, format!(\"{parent}.clicks\"), true)?",
        "s::Value::new(server, format!(\"{parent}.progress\"), 0.0f32, true)?",
        "s::Value::new(server, format!(\"{parent}.level\"), 0.0f64, true)?",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    let python = std::fs::read_to_string(dir.join("python/__init__.py")).unwrap();
    for expected in [
        "self.plain: s.Image = s.Image()\n",
        "self.camera: s.Image = s.Image(low_priority=True)\n",
        "def __init__(self, parent: str, low_priority: bool = False) -> None:",
        "self.inner: ModesInner = ModesInner(parent + \".inner\", low_priority=True)\n",
        "self.ordered: ModesInner = ModesInner(parent + \".ordered\")\n",
        "s.Data[np.uint8] = s.Data(np.uint8, low_priority=low_priority)\n",
        "self.preview: s.Image = s.Image(low_priority=True)\n",
        "self.leaf: ModesLeaf = ModesLeaf(parent + \".leaf\")\n",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }
    // The group without bulk states keeps its plain constructor.
    assert_eq!(python.matches("low_priority: bool = False").count(), 1);
    for queued in ["self.clicks", "self.progress", "self.level"] {
        let line = python.lines().find(|line| line.contains(queued)).unwrap();
        assert!(line.ends_with(", True)"), "{line}");
    }

    let hash = |modes: bool| match modes {
        true => ClientBuilder::<Modes>::new(egui::Context::default(), 0).get_version_hash(),
        false => ClientBuilder::<NoModes>::new(egui::Context::default(), 0).get_version_hash(),
    };
    assert_eq!(hash(true), hash(false));
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        hash(true)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Data states with headers.
struct WithHeaders;

impl State for WithHeaders {
    const NAME: &'static str = "WithHeaders";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Data<f32, GeneratedOuter> = c.data("data");
        let _: DataMulti<u8, GeneratedEnum> = c.data_multi("data_multi");
        let _: DataTake<u16, (u32, String)> = c.data_take("data_take");
        let _: DataMultiTake<i64, Vec<u8>> = c.data_multi_take("data_multi_take");
        let _: Data<u8> = c.data("plain");
        Self
    }
}

#[test]
fn bindings_carry_data_header_types() {
    let dir = output_dir("with_headers", 0);
    generate_rust::<WithHeaders>(&dir).unwrap();
    generate_python::<WithHeaders>(dir.join("python")).unwrap();
    let rust = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    let structs = std::fs::read_to_string(dir.join("structs.rs")).unwrap();
    let enums = std::fs::read_to_string(dir.join("enums.rs")).unwrap();
    for expected in [
        "s::Data<f32, structs::GeneratedOuter>",
        "s::DataMulti<u8, enums::GeneratedEnum>",
        "s::DataTake<u16, (u32, ::std::string::String)>",
        "s::DataMultiTake<i64, ::std::vec::Vec<u8>>",
        "s::Data<u8>",
        "s::Data::<f32, structs::GeneratedOuter>::new(",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    assert!(structs.contains("pub struct GeneratedOuter"));
    assert!(structs.contains("pub struct GeneratedInner"));
    assert!(enums.contains("pub enum GeneratedEnum"));

    let python = std::fs::read_to_string(dir.join("python/__init__.py")).unwrap();
    for expected in [
        "s.DataWithHeader[np.float32, structs.GeneratedOuter] = s.DataWithHeader(np.float32, ",
        "s.DataMultiWithHeader[np.uint8, enums.GeneratedEnum] = s.DataMultiWithHeader(np.uint8, ",
        "s.DataTakeWithHeader[np.uint16, tuple[int, str]] = s.DataTakeWithHeader(np.uint16, ",
        "s.DataMultiTakeWithHeader[np.int64, list[int]] = s.DataMultiTakeWithHeader(np.int64, ",
        "s.Data[np.uint8] = s.Data(np.uint8)",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }

    let client = ClientBuilder::<WithHeaders>::new(egui::Context::default(), 0).get_version_hash();
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        client
    );
    assert_eq!(generated_hash(&python, "VERSION_HASH: int = "), client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Images with and without headers, one of them low priority.
#[derive(State)]
#[allow(dead_code)]
struct WithImageHeaders {
    image: Image<GeneratedOuter>,
    #[state(low_priority)]
    images: ImageMulti<(u32, String)>,
    plain: Image,
}

#[test]
fn bindings_carry_image_header_types() {
    let dir = output_dir("with_image_headers", 0);
    generate_rust::<WithImageHeaders>(&dir).unwrap();
    generate_python::<WithImageHeaders>(dir.join("python")).unwrap();
    let rust = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    let structs = std::fs::read_to_string(dir.join("structs.rs")).unwrap();
    for expected in [
        "pub image: s::Image<structs::GeneratedOuter>,",
        "pub images: s::ImageMulti<(u32, ::std::string::String)>,",
        "pub plain: s::Image,",
        "s::Image::<structs::GeneratedOuter>::new(server, format!(\"{parent}.image\"), false)?",
        "s::ImageMulti::<(u32, ::std::string::String)>::new(server, format!(\"{parent}.images\"), true)?",
        "s::Image::new(server, format!(\"{parent}.plain\"), false)?",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    assert!(structs.contains("pub struct GeneratedOuter"));

    let python = std::fs::read_to_string(dir.join("python/__init__.py")).unwrap();
    for expected in [
        "self.image: s.ImageWithHeader[structs.GeneratedOuter] = s.ImageWithHeader(",
        "self.images: s.ImageMultiWithHeader[tuple[int, str]] = s.ImageMultiWithHeader(",
        ", low_priority=True)",
        "self.plain: s.Image = s.Image()",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }

    // A header changes the layout hash; an image without one keeps it.
    let client =
        ClientBuilder::<WithImageHeaders>::new(egui::Context::default(), 0).get_version_hash();
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        client
    );
    assert_eq!(generated_hash(&python, "VERSION_HASH: int = "), client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An enum with data: a unit variant, named fields with a struct, tuple
/// variants with a fieldless enum and with two fields, and a variant whose
/// name Python uses for the union of the variants.
#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
enum GeneratedCommand {
    #[default]
    Stop,
    MoveTo {
        x: f32,
        inner: GeneratedInner,
    },
    Wrap(GeneratedEnum),
    Pair(u8, String),
    Type,
}

#[derive(State)]
#[allow(dead_code)]
struct WithDataEnums {
    #[state(init = GeneratedCommand::MoveTo { x: 0.5, inner: GeneratedInner { enabled: true } })]
    command: Value<GeneratedCommand>,
    #[state(init = GeneratedCommand::Pair(3, "three".into()))]
    fixed: Static<GeneratedCommand>,
    #[state(queue)]
    commands: Signal<GeneratedCommand>,
    data: Data<u8, GeneratedCommand>,
}

#[test]
fn bindings_carry_enums_with_data() {
    let dir = output_dir("with_data_enums", 0);
    generate_rust::<WithDataEnums>(&dir).unwrap();
    generate_python::<WithDataEnums>(dir.join("python")).unwrap();
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();

    let (rust, enums) = (read("mod.rs"), read("enums.rs"));
    for expected in [
        "s::Value::new(server, format!(\"{parent}.command\"), enums::GeneratedCommand::MoveTo { x: 0.5f32, inner: structs::GeneratedInner { enabled: true } }, false)?",
        "s::Static::new(server, format!(\"{parent}.fixed\"), enums::GeneratedCommand::Pair(3u8, ::std::string::String::from(\"three\")))?",
        "pub data: s::Data<u8, enums::GeneratedCommand>,",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    for expected in [
        "#[derive(Clone, Debug, PartialEq)]\npub enum GeneratedCommand {",
        "    Stop,\n",
        "    MoveTo {\n        /// Generated `x` field.\n        x: f32,\n        /// Generated `inner` field.\n        inner: super::structs::GeneratedInner,\n    },",
        "    Wrap(GeneratedEnum),",
        "    Pair(u8, ::std::string::String),",
        "    Type,\n",
        "pub enum GeneratedEnum {",
    ] {
        assert!(enums.contains(expected), "missing {expected} in\n{enums}");
    }

    let (python, module) = (read("python/__init__.py"), read("python/enums.py"));
    assert!(!dir.join("python/enums.pyi").exists());
    // A value is annotated with the union of the variants, and a unit variant
    // is created like the others.
    for expected in [
        "s.Value[enums.GeneratedCommand.Type](0, enums.GeneratedCommand.MoveTo(0.5, structs.GeneratedInner(True)))",
        "s.Static[enums.GeneratedCommand.Type](0, enums.GeneratedCommand.Pair(3, \"three\"))",
        "self.commands: s.Signal[enums.GeneratedCommand.Type] = s.Signal[enums.GeneratedCommand.Type](0, True)",
        "s.DataWithHeader[np.uint8, enums.GeneratedCommand.Type]",
        "s.denu(enums.GeneratedCommand, [s.vunit(\"Stop\", enums.GeneratedCommand.Stop), s.vnamed(\"MoveTo\", [s.f32, s.cl([s.bo], structs.GeneratedInner)], enums.GeneratedCommand.MoveTo), s.vtuple(\"Wrap\", [s.enu(enums.GeneratedEnum)], enums.GeneratedCommand.Wrap), s.vtuple(\"Pair\", [s.u8, s.st], enums.GeneratedCommand.Pair), s.vunit(\"Type\", enums.GeneratedCommand.Type_)])",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }
    // Every variant is a frozen dataclass nested in the enum; the variant
    // `Type` makes way for the union of the variants.
    for expected in [
        "from __future__ import annotations\n",
        // The module's own imports have private names, which no enum or
        // variant can take.
        "from dataclasses import dataclass as _dataclass\nfrom enum import IntEnum as _IntEnum\nfrom typing import final as _final\n\nimport egui_states.structures as _s\n\nfrom . import structs\n",
        "class GeneratedEnum(_IntEnum):",
        "class GeneratedCommand(_s._CustomEnum):",
        "    @_final\n    @_dataclass(frozen=True, slots=True)\n    class Stop:\n        \"\"\"Protocol variant ``GeneratedCommand.Stop``.\"\"\"\n\n    @_final",
        "    @_dataclass(frozen=True, slots=True)\n    class MoveTo:\n",
        "        x: float\n        inner: structs.GeneratedInner\n",
        "    @_dataclass(frozen=True, slots=True, repr=False)\n    class Wrap(_s._TupleVariant):\n",
        "        _0: GeneratedEnum\n",
        "    class Pair(_s._TupleVariant):\n",
        "        _0: int\n        _1: str\n",
        "    class Type_:\n",
        "    type Type = Stop | MoveTo | Wrap | Pair | Type_\n",
    ] {
        assert!(module.contains(expected), "missing {expected} in\n{module}");
    }

    let client =
        ClientBuilder::<WithDataEnums>::new(egui::Context::default(), 0).get_version_hash();
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        client
    );
    assert_eq!(generated_hash(&python, "VERSION_HASH: int = "), client);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_an_image_header_changes_the_layout_hash() {
    #[derive(State)]
    #[allow(dead_code)]
    struct Plain {
        image: Image,
        images: ImageMulti,
    }
    #[derive(State)]
    #[allow(dead_code)]
    struct Headers {
        image: Image<u32>,
        images: ImageMulti<u32>,
    }
    #[derive(State)]
    #[allow(dead_code)]
    struct Unit {
        image: Image<()>,
        images: ImageMulti<()>,
    }
    fn hash<S: State>() -> u64 {
        ClientBuilder::<S>::new(egui::Context::default(), 0).get_version_hash()
    }
    let (plain, headers, unit) = (hash::<Plain>(), hash::<Headers>(), hash::<Unit>());
    assert_ne!(plain, headers);
    assert_eq!(plain, unit);
}

/// File transfer endpoints with and without metadata.
#[derive(State)]
#[allow(dead_code)]
struct WithTransfers {
    files: Download,
    results: Download<GeneratedOuter>,
    logs: Upload,
    inputs: Upload<GeneratedEnum>,
}

#[test]
fn bindings_carry_transfer_endpoints() {
    let dir = output_dir("with_transfers", 0);
    generate_rust::<WithTransfers>(&dir).unwrap();
    generate_python::<WithTransfers>(dir.join("python")).unwrap();
    let rust = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    let structs = std::fs::read_to_string(dir.join("structs.rs")).unwrap();
    for expected in [
        "pub files: s::Download,",
        "pub results: s::Download<structs::GeneratedOuter>,",
        "pub logs: s::Upload,",
        "pub inputs: s::Upload<enums::GeneratedEnum>,",
        "s::Download::<()>::new(server, format!(\"{parent}.files\"))?",
        "s::Upload::<enums::GeneratedEnum>::new(server, format!(\"{parent}.inputs\"))?",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    assert!(structs.contains("pub struct GeneratedOuter"));

    let python = std::fs::read_to_string(dir.join("python/__init__.py")).unwrap();
    for expected in [
        "self.files: s.Download = s.Download()",
        "self.results: s.DownloadWithMeta[structs.GeneratedOuter] = s.DownloadWithMeta(",
        "self.logs: s.Upload = s.Upload()",
        "self.inputs: s.UploadWithMeta[enums.GeneratedEnum] = s.UploadWithMeta(",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }

    let client =
        ClientBuilder::<WithTransfers>::new(egui::Context::default(), 0).get_version_hash();
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        client
    );
    assert_eq!(generated_hash(&python, "VERSION_HASH: int = "), client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pins the kind tags and metadata type ids of the transfer endpoints.
#[test]
fn layout_hash_of_transfer_endpoints_is_stable() {
    const GOLDEN: u64 = 2088561202456885291;
    assert_eq!(
        ClientBuilder::<WithTransfers>::new(egui::Context::default(), 0).get_version_hash(),
        GOLDEN
    );
}

struct Golden;

impl State for Golden {
    const NAME: &'static str = "Golden";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("value", 0);
        let _: Signal<bool> = c.signal("signal");
        Self
    }
}

/// Peers built from different releases compare this hash during the handshake.
#[test]
fn layout_hash_of_a_fixed_schema_is_stable() {
    const GOLDEN: u64 = 15207089630600656803;
    assert_eq!(
        ClientBuilder::<Golden>::new(egui::Context::default(), 0).get_version_hash(),
        GOLDEN
    );
}

/// Pins the per-kind tags and type ids of every state kind, so a refactor of
/// registration cannot change the hash of any of them.
#[test]
fn layout_hash_of_a_schema_with_every_kind_is_stable() {
    const GOLDEN: u64 = 11746098588710645841;
    assert_eq!(
        ClientBuilder::<AllKinds>::new(egui::Context::default(), 0).get_version_hash(),
        GOLDEN
    );
}

struct NamedLikeServer;

impl State for NamedLikeServer {
    const NAME: &'static str = "StatesServer";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("value", 0);
        Self
    }
}

struct NamedLikeModule;

impl State for NamedLikeModule {
    const NAME: &'static str = "enums";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("value", 0);
        Self
    }
}

struct ContainsModuleName;

impl State for ContainsModuleName {
    const NAME: &'static str = "ContainsModuleName";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: NamedLikeModule = c.substate("nested");
        Self
    }
}

#[test]
#[should_panic(expected = "State StatesServer conflicts with a generated symbol")]
fn a_root_state_named_like_the_generated_server_is_rejected() {
    let _ = generate_rust::<NamedLikeServer>(output_dir("named_like_server", 0));
}

#[test]
#[should_panic(expected = "State enums conflicts with a generated symbol")]
fn a_nested_state_named_like_a_generated_module_is_rejected() {
    let _ = generate_python::<ContainsModuleName>(output_dir("named_like_module", 0));
}

mod first_shape {
    #[egui_states::typed]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    pub(super) struct Shape {
        pub(super) value: bool,
    }

    #[egui_states::typed]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    pub(super) enum Mode {
        #[default]
        Off,
    }
}

mod second_shape {
    #[egui_states::typed]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    pub(super) struct Shape {
        pub(super) value: u8,
    }

    #[egui_states::typed]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    pub(super) enum Mode {
        #[default]
        Off,
        On,
    }
}

struct ConflictingStructs;

impl State for ConflictingStructs {
    const NAME: &'static str = "ConflictingStructs";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<first_shape::Shape> = c.value("first", Default::default());
        let _: Value<second_shape::Shape> = c.value("second", Default::default());
        Self
    }
}

struct ConflictingEnums;

impl State for ConflictingEnums {
    const NAME: &'static str = "ConflictingEnums";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<first_shape::Mode> = c.value("first", Default::default());
        let _: Value<second_shape::Mode> = c.value("second", Default::default());
        Self
    }
}

#[test]
#[should_panic(expected = "Struct Shape defined multiple times with different fields")]
fn one_struct_name_with_different_fields_is_rejected() {
    let _ = generate_python::<ConflictingStructs>(output_dir("conflicting_structs", 0));
}

#[test]
#[should_panic(expected = "Enum Mode defined multiple times with different variants")]
fn one_enum_name_with_different_variants_is_rejected() {
    let _ = generate_python::<ConflictingEnums>(output_dir("conflicting_enums", 0));
}

struct KeywordSubstate;

impl State for KeywordSubstate {
    const NAME: &'static str = "KeywordSubstate";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Leaf = c.substate("global");
        Self
    }
}

/// The attribute is escaped, but the state path must keep the client's name.
#[test]
fn a_keyword_substate_keeps_its_wire_path() {
    let dir = output_dir("keyword_substate", 0);
    generate_python::<KeywordSubstate>(&dir).unwrap();
    let output = std::fs::read_to_string(dir.join("__init__.py")).unwrap();
    assert!(
        output.contains("    __egui_names__ = {\"global_\": \"global\"}\n"),
        "{output}"
    );
    assert!(
        output.contains("self.global_: Leaf = Leaf(parent + \".global\")"),
        "{output}"
    );
}

struct EscapedNameCollision;

impl State for EscapedNameCollision {
    const NAME: &'static str = "EscapedNameCollision";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<u8> = c.value("from", 0);
        let _: Value<u8> = c.value("from_", 0);
        Self
    }
}

#[test]
#[should_panic(
    expected = "EscapedNameCollision: the Python name `from_` of `from` conflicts with another name"
)]
fn an_escaped_name_that_collides_is_rejected() {
    let _ = generate_python::<EscapedNameCollision>(output_dir("escaped_collision", 0));
}

/// `Type` is spelled `Type_` in Python, which this enum already uses.
#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
#[allow(non_camel_case_types)]
enum CollidingVariants {
    #[default]
    Type,
    Type_,
    Data(u8),
}

#[derive(State)]
#[allow(dead_code)]
struct WithCollidingVariants {
    value: Value<CollidingVariants>,
}

#[test]
#[should_panic(expected = "CollidingVariants: two variants have the Python name `Type_`")]
fn a_variant_that_collides_with_an_escaped_one_is_rejected() {
    let _ = generate_python::<WithCollidingVariants>(output_dir("variant_collision", 0));
}

#[derive(State)]
#[allow(dead_code)]
struct WithoutCustomTypes {
    count: Value<i32>,
    reset: Signal<()>,
}

#[test]
fn enum_and_struct_files_exist_only_for_a_state_that_has_such_types() {
    let dir = output_dir("without_custom_types", 0);
    let python = dir.join("python");
    std::fs::create_dir_all(&python).unwrap();
    // An earlier run left generated files behind; `notes.py` and a
    // hand-written `structs.rs` are somebody else's.
    let stale = "# Generated by build.rs, do not edit\nfrom enum import IntEnum\n";
    std::fs::write(python.join("enums.py"), stale).unwrap();
    std::fs::write(python.join("structs.py"), stale).unwrap();
    std::fs::write(
        dir.join("enums.rs"),
        "//! Enums.\n\n// Generated by build.rs, do not edit\n",
    )
    .unwrap();
    std::fs::write(dir.join("structs.rs"), "// written by hand\n").unwrap();

    generate_rust::<WithoutCustomTypes>(&dir).unwrap();
    generate_python::<WithoutCustomTypes>(&python).unwrap();

    assert!(!python.join("enums.py").exists());
    assert!(!python.join("structs.py").exists());
    assert!(!dir.join("enums.rs").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("structs.rs")).unwrap(),
        "// written by hand\n"
    );
    let init = std::fs::read_to_string(python.join("__init__.py")).unwrap();
    assert!(!init.contains("from . import"), "{init}");
    let module = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    assert!(!module.contains("pub mod"), "{module}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_state_with_only_enums_or_only_structs_imports_just_that_module() {
    #[derive(State)]
    #[allow(dead_code)]
    struct OnlyEnums {
        choice: Value<GeneratedEnum>,
    }
    #[derive(State)]
    #[allow(dead_code)]
    struct OnlyStructs {
        inner: Value<GeneratedInner>,
    }

    let dir = output_dir("only_enums", 0);
    generate_rust::<OnlyEnums>(&dir).unwrap();
    generate_python::<OnlyEnums>(dir.join("python")).unwrap();
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();
    assert!(read("python/__init__.py").contains("\nfrom . import enums\n"));
    assert!(read("python/enums.py").contains("\nfrom enum import IntEnum as _IntEnum\n"));
    assert!(
        read("mod.rs").contains("pub mod enums;") && !read("mod.rs").contains("pub mod structs;")
    );
    assert!(dir.join("enums.rs").exists() && !dir.join("structs.rs").exists());
    assert!(dir.join("python/enums.py").exists() && !dir.join("python/structs.py").exists());
    let _ = std::fs::remove_dir_all(&dir);

    let dir = output_dir("only_structs", 0);
    generate_rust::<OnlyStructs>(&dir).unwrap();
    generate_python::<OnlyStructs>(dir.join("python")).unwrap();
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();
    assert!(read("python/__init__.py").contains("\nfrom . import structs\n"));
    assert!(!read("python/structs.py").contains("from . import enums"));
    assert!(
        read("mod.rs").contains("pub mod structs;") && !read("mod.rs").contains("pub mod enums;")
    );
    assert!(dir.join("structs.rs").exists() && !dir.join("enums.rs").exists());
    assert!(dir.join("python/structs.py").exists() && !dir.join("python/enums.py").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The message of the panic with which `generate` rejects a state tree, after
/// checking that it wrote nothing into `dir`.
fn rejection(dir: PathBuf, generate: impl FnOnce(&PathBuf) -> Result<(), String>) -> String {
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| generate(&dir)))
        .expect_err("the state tree was generated");
    assert!(!dir.exists(), "{} was written", dir.display());
    match panic.downcast::<String>() {
        Ok(message) => *message,
        Err(panic) => panic
            .downcast::<&str>()
            .map(|message| message.to_string())
            .expect("a panic message"),
    }
}

/// The messages of both generators rejecting the state tree `S`.
fn rejections<S: State>(name: &str) -> [String; 2] {
    [
        rejection(output_dir(name, 0), |dir| generate_python::<S>(dir)),
        rejection(output_dir(name, 1), |dir| generate_rust::<S>(dir)),
    ]
}

/// A unit in a variant, as in `Cmd::A(())`.
#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
enum UnitInVariant {
    #[default]
    Stop,
    Empty(()),
}

#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
enum UnitInNamedVariant {
    #[default]
    Stop,
    Empty {
        nothing: (),
    },
}

#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
struct UnitInStruct {
    nothing: (),
}

/// A state tree with the one state `state`, of the handle type `H`.
struct OneState<H>(std::marker::PhantomData<H>);

macro_rules! one_state {
    ($($handle:ty => |$c:ident| $create:expr),* $(,)?) => {$(
        impl State for OneState<$handle> {
            const NAME: &'static str = "OneState";

            fn new($c: &mut impl StatesCreator) -> Self {
                let _: $handle = $create;
                Self(std::marker::PhantomData)
            }
        }
    )*};
}

one_state! {
    Value<UnitInVariant> => |c| c.value("state", Default::default()),
    Static<UnitInNamedVariant> => |c| c.add_static("state", Default::default()),
    Signal<UnitInStruct> => |c| c.signal("state"),
    ValueTake<Option<()>> => |c| c.value_take("state"),
    VecState<(u8, ())> => |c| c.vec("state"),
    MapState<u8, Vec<()>> => |c| c.map("state"),
    Data<u8, [(); 2]> => |c| c.data("state"),
    Image<HashMap<u8, ()>> => |c| c.image("state"),
    Download<Option<()>> => |c| c.download("state"),
    Value<()> => |c| c.value("state", ()),
    Static<()> => |c| c.add_static("state", ()),
    VecState<()> => |c| c.vec("state"),
    MapState<(), u8> => |c| c.map("state"),
    MapState<u8, ()> => |c| c.map("state"),
}

/// A unit has no Python spelling as a field or an element (`_0: ` is a
/// syntax error), so both generators reject it before they write a file.
#[test]
fn a_unit_inside_another_type_is_rejected() {
    let nested = [
        rejections::<OneState<Value<UnitInVariant>>>("unit_in_variant"),
        rejections::<OneState<Static<UnitInNamedVariant>>>("unit_in_named_variant"),
        rejections::<OneState<Signal<UnitInStruct>>>("unit_in_struct"),
        rejections::<OneState<ValueTake<Option<()>>>>("unit_in_option"),
        rejections::<OneState<VecState<(u8, ())>>>("unit_in_tuple"),
        rejections::<OneState<MapState<u8, Vec<()>>>>("unit_in_vec"),
        rejections::<OneState<Data<u8, [(); 2]>>>("unit_in_array"),
        rejections::<OneState<Image<HashMap<u8, ()>>>>("unit_in_map"),
        rejections::<OneState<Download<Option<()>>>>("unit_in_meta"),
    ];
    for message in nested.as_flattened() {
        assert_eq!(
            message,
            "State state: the unit type `()` is not supported inside another type; only a Signal or a ValueTake takes it, as its whole type"
        );
    }
}

/// A stored value and a collection item need a content; their unit used to
/// panic on the initial value or to generate `s.Vec[]`.
#[test]
fn a_unit_as_a_stored_value_or_an_item_is_rejected() {
    let whole = [
        rejections::<OneState<Value<()>>>("unit_value"),
        rejections::<OneState<Static<()>>>("unit_static"),
        rejections::<OneState<VecState<()>>>("unit_vec"),
        rejections::<OneState<MapState<(), u8>>>("unit_map_key"),
        rejections::<OneState<MapState<u8, ()>>>("unit_map_value"),
    ];
    for message in whole.as_flattened() {
        assert_eq!(
            message,
            "State state: the unit type `()` is supported only as the type of a Signal or a ValueTake"
        );
    }
}

/// The unit stays the payload-free signal and take value, and no header or
/// metadata.
#[test]
fn a_unit_is_the_whole_type_of_a_signal_or_a_take_value() {
    #[derive(State)]
    #[allow(dead_code)]
    struct Units {
        signal: Signal<()>,
        take: ValueTake<()>,
        data: Data<u8, ()>,
        image: Image<()>,
        upload: Upload<()>,
    }

    let dir = output_dir("units", 0);
    generate_rust::<Units>(&dir).unwrap();
    generate_python::<Units>(dir.join("python")).unwrap();
    let rust = std::fs::read_to_string(dir.join("mod.rs")).unwrap();
    for expected in ["pub signal: s::Signal<()>,", "pub take: s::ValueTake<()>,"] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    let python = std::fs::read_to_string(dir.join("python/__init__.py")).unwrap();
    for expected in [
        "self.signal: s.SignalEmpty = s.SignalEmpty()\n",
        "self.take: s.ValueTakeEmpty = s.ValueTakeEmpty()\n",
        "self.data: s.Data[np.uint8] = s.Data(np.uint8)\n",
        "self.image: s.Image = s.Image()\n",
        "self.upload: s.Upload = s.Upload()\n",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Raw identifiers as struct fields.
#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
struct RawStruct {
    r#type: u8,
    r#for: bool,
}

/// Raw identifiers as the variants of a fieldless enum.
#[egui_states::typed]
#[derive(Clone, Copy, Default, PartialEq, egui_states::InitialValue)]
#[allow(non_camel_case_types)]
enum RawEnum {
    #[default]
    r#type,
    r#in,
}

/// Raw identifiers as the variants of an enum with data and as their fields.
#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
#[allow(non_camel_case_types)]
enum RawCommand {
    #[default]
    Stop,
    r#match {
        r#type: u8,
        r#if: bool,
    },
    r#try(u8),
}

/// Raw identifiers as state names, of a value and of a nested group.
#[derive(State)]
#[allow(dead_code)]
struct WithRawNames {
    #[state(init = RawStruct { r#type: 4, r#for: true })]
    r#type: Value<RawStruct>,
    #[state(init = RawEnum::r#in)]
    r#while: Static<RawEnum>,
    #[state(init = RawCommand::r#match { r#type: 1, r#if: true })]
    command: Value<RawCommand>,
    r#loop: ModesLeaf,
}

/// Python gets the name without `r#`, escaped when it is a Python keyword;
/// Rust and the wire keep the raw identifier.
#[test]
fn raw_identifiers_lose_their_prefix_only_in_python() {
    let dir = output_dir("raw_names", 0);
    generate_rust::<WithRawNames>(&dir).unwrap();
    generate_python::<WithRawNames>(dir.join("python")).unwrap();
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();

    let python = read("python/__init__.py");
    for expected in [
        "    __egui_names__ = {\"type\": \"r#type\", \"while_\": \"r#while\", \"loop\": \"r#loop\"}\n",
        "self.type: s.Value[structs.RawStruct] = s.Value[structs.RawStruct](0, structs.RawStruct(4, True))\n",
        "self.while_: s.Static[enums.RawEnum] = s.Static[enums.RawEnum](1, enums.RawEnum.in_)\n",
        "(2, enums.RawCommand.match(1, True))\n",
        "self.loop: ModesLeaf = ModesLeaf(parent + \".r#loop\")\n",
        // The variants keep their wire names, which the type hash covers.
        "s.denu(enums.RawCommand, [s.vunit(\"Stop\", enums.RawCommand.Stop), s.vnamed(\"r#match\", [s.u8, s.bo], enums.RawCommand.match), s.vtuple(\"r#try\", [s.u8], enums.RawCommand.try_)])",
    ] {
        assert!(python.contains(expected), "missing {expected} in\n{python}");
    }
    let structs = read("python/structs.py");
    let expected = "    type: int\n    for_: bool\n    __egui_names__ = {\"type\": \"r#type\", \"for_\": \"r#for\"}\n";
    assert!(structs.contains(expected), "{structs}");
    let enums = read("python/enums.py");
    for expected in [
        "class RawEnum(_IntEnum):\n    \"\"\"A generated protocol enumeration.\"\"\"\n    type = 0\n    in_ = 1\n    __egui_names__ = {\"type\": \"r#type\", \"in_\": \"r#in\"}\n",
        "    class match:\n        \"\"\"Protocol variant ``RawCommand.match``.\"\"\"\n\n        type: int\n        if_: bool\n        __egui_names__ = {\"type\": \"r#type\", \"if_\": \"r#if\"}\n",
        "    class try_(_s._TupleVariant):\n",
        "    type Type = Stop | match | try_\n",
    ] {
        assert!(enums.contains(expected), "missing {expected} in\n{enums}");
    }
    // Outside of the wire names, which are text, no `r#` is left.
    for generated in [&python, &structs, &enums] {
        let code = generated.replace("\"r#", "").replace(".r#", "");
        assert!(!code.contains("r#"), "{generated}");
    }

    let rust = read("mod.rs");
    for expected in [
        "pub r#type: s::Value<structs::RawStruct>,",
        "r#type: s::Value::new(server, format!(\"{parent}.r#type\"), structs::RawStruct { r#type: 4u8, r#for: true }, false)?",
        "s::Static::new(server, format!(\"{parent}.r#while\"), enums::RawEnum::r#in)?",
        "enums::RawCommand::r#match { r#type: 1u8, r#if: true }",
        "r#loop: ModesLeaf::new(server, &format!(\"{parent}.r#loop\"))?",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }

    let client = ClientBuilder::<WithRawNames>::new(egui::Context::default(), 0).get_version_hash();
    assert_eq!(
        generated_hash(&rust, "pub const VERSION_HASH: u64 = "),
        client
    );
    assert_eq!(generated_hash(&python, "VERSION_HASH: int = "), client);
    let _ = std::fs::remove_dir_all(&dir);
}

/// State names given as text, the way a handwritten `State` does.
struct Named<const CASE: usize>;

const NAME_CASES: [[&str; 2]; 3] = [["r#from", "from_"], ["r#type", "type"], ["r#in", "in"]];

impl<const CASE: usize> State for Named<CASE> {
    const NAME: &'static str = "Named";

    fn new(c: &mut impl StatesCreator) -> Self {
        for name in NAME_CASES[CASE] {
            let _: Value<u8> = c.value(name, 0);
        }
        Self
    }
}

/// The names stay distinct on the wire, but not in Python.
#[test]
fn a_raw_identifier_that_collides_in_python_is_rejected() {
    let python = |name: &str, generate: fn(&PathBuf) -> Result<(), String>| {
        rejection(output_dir(name, 0), generate)
    };
    assert_eq!(
        python("raw_escaped", |dir| generate_python::<Named<0>>(dir)),
        "Named: the Python name `from_` of `r#from` conflicts with another name"
    );
    assert_eq!(
        python("raw_plain", |dir| generate_python::<Named<1>>(dir)),
        "Named: the Python name `type` of `r#type` conflicts with another name"
    );
    assert_eq!(
        python("raw_keyword", |dir| generate_python::<Named<2>>(dir)),
        "Named: the Python name `in_` of `r#in` conflicts with another name"
    );
}

/// Fixed-size arrays at every depth of a field type.
#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
struct FixedSizes {
    plain: [u8; 3],
    nested: [[u8; 3]; 2],
    listed: Vec<[u8; 3]>,
    optional: Option<[u8; 3]>,
    pair: ([u8; 2], Vec<[f32; 4]>),
    table: HashMap<u8, [u16; 5]>,
    free: Vec<u8>,
}

/// The length of a fixed-size array is a comment, which belongs after the
/// whole annotation: inside the brackets it would swallow the rest of it.
#[test]
fn fixed_size_comments_end_the_line_of_a_struct_field() {
    #[derive(State)]
    #[allow(dead_code)]
    struct WithFixedSizes {
        sizes: Value<FixedSizes>,
    }

    let dir = output_dir("fixed_sizes", 0);
    generate_python::<WithFixedSizes>(&dir).unwrap();
    let structs = std::fs::read_to_string(dir.join("structs.py")).unwrap();
    let expected = "    plain: list[int]  # fixed size 3
    nested: list[list[int]]  # fixed sizes 2, 3
    listed: list[list[int]]  # fixed size 3
    optional: list[int] | None  # fixed size 3
    pair: tuple[list[int], list[list[float]]]  # fixed sizes 2, 4
    table: dict[int, list[int]]  # fixed size 5
    free: list[int]
";
    assert!(structs.contains(expected), "{structs}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Types named like what the generated modules use themselves.
mod shadowing {
    #[egui_states::typed]
    #[derive(Clone, Copy)]
    pub(super) enum Option {
        Off,
        On,
    }

    #[egui_states::typed]
    #[derive(Clone, Copy)]
    #[allow(non_camel_case_types)]
    pub(super) enum s {
        First,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) enum String {
        s,
        dataclass(u8),
        r#final {
            text: ::std::string::String,
            maybe: ::std::option::Option<s>,
            choices: ::std::vec::Vec<Option>,
        },
        IntEnum(::std::collections::HashMap<u8, ::std::string::String>),
    }

    #[egui_states::typed]
    #[derive(Clone)]
    pub(super) struct Vector {
        pub(super) option: ::std::option::Option<Option>,
        pub(super) string: ::std::string::String,
    }
}

/// An enum called `Option` or `String` shadows the prelude of the generated
/// Rust module, and `s`, `dataclass`, `final` and `IntEnum` are what the
/// generated `enums.py` imports. Neither may change what the generated code
/// refers to.
#[test]
fn generated_names_do_not_shadow_what_the_generated_code_uses() {
    #[derive(State)]
    #[allow(dead_code)]
    struct WithShadowing {
        option: Signal<shadowing::Option>,
        short: Signal<shadowing::s>,
        string: Signal<shadowing::String>,
        vector: Signal<shadowing::Vector>,
    }

    let dir = output_dir("shadowing", 0);
    generate_rust::<WithShadowing>(&dir).unwrap();
    generate_python::<WithShadowing>(dir.join("python")).unwrap();
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();

    let enums = read("enums.rs");
    for expected in [
        "pub enum Option {",
        "pub enum String {",
        "        text: ::std::string::String,\n",
        "        maybe: ::std::option::Option<s>,\n",
        "        choices: ::std::vec::Vec<Option>,\n",
        "    IntEnum(::std::collections::HashMap<u8, ::std::string::String>),\n",
    ] {
        assert!(enums.contains(expected), "missing {expected} in\n{enums}");
    }
    let structs = read("structs.rs");
    for expected in [
        "    pub option: ::std::option::Option<super::enums::Option>,\n",
        "    pub string: ::std::string::String,\n",
    ] {
        assert!(
            structs.contains(expected),
            "missing {expected} in\n{structs}"
        );
    }

    let module = read("python/enums.py");
    for expected in [
        "class Option(_IntEnum):",
        "class s(_IntEnum):",
        "class String(_s._CustomEnum):",
        "    @_final\n    @_dataclass(frozen=True, slots=True)\n    class s:\n",
        "    @_final\n    @_dataclass(frozen=True, slots=True, repr=False)\n    class dataclass(_s._TupleVariant):\n",
        "    @_final\n    @_dataclass(frozen=True, slots=True)\n    class final:\n",
        "    @_final\n    @_dataclass(frozen=True, slots=True, repr=False)\n    class IntEnum(_s._TupleVariant):\n",
        "    type Type = s | dataclass | final | IntEnum\n",
    ] {
        assert!(module.contains(expected), "missing {expected} in\n{module}");
    }
    // Nothing is left of the unaliased imports.
    for import in [
        "import dataclass\n",
        "import IntEnum\n",
        "import final\n",
        " as s\n",
    ] {
        assert!(!module.contains(import), "{import:?} in\n{module}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Types whose names Python cannot define, or which the generated modules
/// still bind themselves.
mod unusable_names {
    #[egui_states::typed]
    #[derive(Clone)]
    pub(super) enum None {
        A,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    pub(super) enum True {
        A(u8),
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) struct lambda {
        pub(super) value: u8,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) enum r#type {
        A,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) enum structs {
        A,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) enum _s {
        A(u8),
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) struct dataclass {
        pub(super) value: u8,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) struct s {
        pub(super) value: u8,
    }

    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) struct enums {
        pub(super) value: u8,
    }

    /// A variant named like an alias `enums.py` imports under.
    #[egui_states::typed]
    #[derive(Clone)]
    #[allow(non_camel_case_types)]
    pub(super) enum Variants {
        A(u8),
        _final,
    }
}

one_state! {
    Signal<unusable_names::None> => |c| c.signal("state"),
    Signal<unusable_names::True> => |c| c.signal("state"),
    Signal<Vec<unusable_names::lambda>> => |c| c.signal("state"),
    Signal<unusable_names::r#type> => |c| c.signal("state"),
    Signal<unusable_names::structs> => |c| c.signal("state"),
    Signal<unusable_names::_s> => |c| c.signal("state"),
    Signal<unusable_names::dataclass> => |c| c.signal("state"),
    Signal<unusable_names::s> => |c| c.signal("state"),
    Signal<unusable_names::enums> => |c| c.signal("state"),
    Signal<unusable_names::Variants> => |c| c.signal("state"),
}

/// The message of the Python generator rejecting the state tree `S`, which
/// the Rust generator accepts.
fn python_rejection<S: State>(name: &str) -> String {
    let dir = output_dir(name, 1);
    generate_rust::<S>(&dir).unwrap();
    assert!(dir.join("mod.rs").exists());
    let _ = std::fs::remove_dir_all(&dir);
    rejection(output_dir(name, 0), |dir| generate_python::<S>(dir))
}

/// A class name is not on the wire, so it has no escaped spelling: a type
/// named like a Python keyword is rejected for Python, where it cannot be
/// defined, and kept for Rust, where it is a plain name.
#[test]
fn type_names_that_are_python_keywords_are_rejected_for_python_only() {
    use unusable_names as names;
    for (message, expected) in [
        (
            python_rejection::<OneState<Signal<names::None>>>("keyword_enum"),
            "Enum None is named like a Python keyword",
        ),
        (
            python_rejection::<OneState<Signal<names::True>>>("keyword_data_enum"),
            "Enum True is named like a Python keyword",
        ),
        (
            python_rejection::<OneState<Signal<Vec<names::lambda>>>>("keyword_struct"),
            "Struct lambda is named like a Python keyword",
        ),
    ] {
        assert_eq!(message, expected);
    }
}

/// A type whose name would replace one its generated module binds, or that
/// has no spelling there, is rejected by both generators: the state
/// definition stays usable from both server languages.
#[test]
fn type_names_that_python_cannot_use_are_rejected() {
    use unusable_names as names;
    for (messages, expected) in [
        (
            rejections::<OneState<Signal<names::r#type>>>("raw_enum"),
            "Enum r#type is named with a raw identifier, which has no Python spelling",
        ),
        (
            rejections::<OneState<Signal<names::structs>>>("module_enum"),
            "Enum structs conflicts with a generated symbol",
        ),
        (
            rejections::<OneState<Signal<names::_s>>>("alias_enum"),
            "Enum _s conflicts with a generated symbol",
        ),
        (
            rejections::<OneState<Signal<names::dataclass>>>("import_struct"),
            "Struct dataclass conflicts with a generated symbol",
        ),
        (
            rejections::<OneState<Signal<names::s>>>("alias_struct"),
            "Struct s conflicts with a generated symbol",
        ),
        (
            rejections::<OneState<Signal<names::enums>>>("module_struct"),
            "Struct enums conflicts with a generated symbol",
        ),
    ] {
        for message in messages {
            assert_eq!(message, expected);
        }
    }

    // Only the Python module has the alias a variant could replace.
    let message = rejection(output_dir("alias_variant", 0), |dir| {
        generate_python::<OneState<Signal<names::Variants>>>(dir)
    });
    assert_eq!(
        message,
        "Variants: the variant `_final` conflicts with a generated symbol"
    );
}

struct NamedLikeKeyword;

impl State for NamedLikeKeyword {
    const NAME: &'static str = "None";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("value", 0);
        Self
    }
}

#[test]
fn a_state_named_like_a_python_keyword_is_rejected_for_python_only() {
    assert_eq!(
        python_rejection::<NamedLikeKeyword>("keyword_state"),
        "State None is named like a Python keyword"
    );
}

struct NamedLike<const N: usize>;

/// Names the generated `__init__.py` binds or uses bare, which the Rust
/// bindings do not.
const PYTHON_MODULE_NAMES: [&str; 3] = ["np", "Exception", "_Sequence"];

impl<const N: usize> State for NamedLike<N> {
    const NAME: &'static str = PYTHON_MODULE_NAMES[N];

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("value", 0);
        Self
    }
}

/// The imports of the generated `__init__.py` are aliased, so a state group
/// may be called `Sequence`. What stays reserved are the names it cannot
/// alias: NumPy, a builtin it names, and the aliases themselves.
#[test]
fn a_state_named_like_a_name_of_the_python_module_is_rejected_for_python_only() {
    assert_eq!(
        python_rejection::<NamedLike<0>>("numpy_state"),
        "State np conflicts with a generated symbol"
    );
    assert_eq!(
        python_rejection::<NamedLike<1>>("builtin_state"),
        "State Exception conflicts with a generated symbol"
    );
    assert_eq!(
        python_rejection::<NamedLike<2>>("alias_state"),
        "State _Sequence conflicts with a generated symbol"
    );
}
