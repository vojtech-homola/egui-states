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
        "s::DataTake<u16, (u32, String)>",
        "s::DataMultiTake<i64, Vec<u8>>",
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
        "pub images: s::ImageMulti<(u32, String)>,",
        "pub plain: s::Image,",
        "s::Image::<structs::GeneratedOuter>::new(server, format!(\"{parent}.image\"), false)?",
        "s::ImageMulti::<(u32, String)>::new(server, format!(\"{parent}.images\"), true)?",
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
        "s::Static::new(server, format!(\"{parent}.fixed\"), enums::GeneratedCommand::Pair(3u8, String::from(\"three\")))?",
        "pub data: s::Data<u8, enums::GeneratedCommand>,",
    ] {
        assert!(rust.contains(expected), "missing {expected} in\n{rust}");
    }
    for expected in [
        "#[derive(Clone, Debug, PartialEq)]\npub enum GeneratedCommand {",
        "    Stop,\n",
        "    MoveTo {\n        /// Generated `x` field.\n        x: f32,\n        /// Generated `inner` field.\n        inner: super::structs::GeneratedInner,\n    },",
        "    Wrap(GeneratedEnum),",
        "    Pair(u8, String),",
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
        "class GeneratedEnum(IntEnum):",
        "class GeneratedCommand(s._CustomEnum):",
        "    @final\n    @dataclass(frozen=True, slots=True)\n    class Stop:\n        \"\"\"Protocol variant ``GeneratedCommand.Stop``.\"\"\"\n\n    @final",
        "    @dataclass(frozen=True, slots=True)\n    class MoveTo:\n",
        "        x: float\n        inner: structs.GeneratedInner\n",
        "    @dataclass(frozen=True, slots=True, repr=False)\n    class Wrap(s._TupleVariant):\n",
        "        _0: GeneratedEnum\n",
        "    class Pair(s._TupleVariant):\n",
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
