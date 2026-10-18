//! Checks the C registration header written by `generate_c`.
#![cfg(all(feature = "build_scripts", feature = "c_abi"))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui_states::build_scripts::generate_c;
use egui_states::{
    ClientBuilder, Data, Image, MapState, Queue, Signal, State, StatesCreator, Static, Value,
    ValueTake, VecState,
};

#[egui_states::typed]
#[derive(Clone, Copy, Default, PartialEq, egui_states::InitialValue)]
enum Sparse {
    Low = -7,
    #[default]
    Mid = 3,
    High = 201,
}

#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
struct Inner {
    flag: bool,
    level: u16,
}

/// Uses `Inner` only through an `Option`, so its definition must still come first.
#[egui_states::typed]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
struct Outer {
    inner: Option<Inner>,
    choice: Sparse,
}

struct Leaf;

impl State for Leaf {
    const NAME: &'static str = "Leaf";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<bool> = c.value("value", true);
        Self
    }
}

struct Golden;

impl State for Golden {
    const NAME: &'static str = "CGolden";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<i32> = c.value("small", i32::MIN);
        let _: Value<i64> = c.value("big", i64::MIN);
        let _: Value<u64> = c.value("unsigned", u64::MAX);
        let _: Value<f32> = c.value("float", -0.0);
        let _: Value<f64> = c.value("double", f64::NAN);
        let _: Value<String> = c.value("text", "a\"b?\0ž".into());
        let _: Value<Outer, Queue> = c.value(
            "outer",
            Outer {
                inner: Some(Inner {
                    flag: true,
                    level: 7,
                }),
                choice: Sparse::High,
            },
        );
        let _: Static<Vec<Sparse>> = c.add_static("list", vec![Sparse::Low, Sparse::High]);
        let _: Value<HashMap<String, u8>> =
            c.value("labels", HashMap::from([("b".into(), 2), ("a".into(), 1)]));
        let _: MapState<String, (u8, [u16; 2])> = c.map("lookup");
        let _: VecState<Option<u32>> = c.vec("items");
        let _: Signal<()> = c.signal("pulse");
        let _: ValueTake<(u8,)> = c.value_take("take");
        let _: Data<f32> = c.data("samples");
        let _: Image = c.image("image");
        let _: Leaf = c.substate("nested");
        Self
    }
}

fn output_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "egui_states_c_codegen_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn generate<S: State>(name: &str) -> (PathBuf, String) {
    let dir = output_dir(name);
    generate_c::<S>(&dir).unwrap();
    let file = dir.join("c_golden.h");
    let header = std::fs::read_to_string(&file).unwrap_or_default();
    (dir, header)
}

#[test]
fn header_registers_every_state_with_exact_initial_values() {
    let (dir, header) = generate::<Golden>("golden");
    for expected in [
        "#ifndef C_GOLDEN_H",
        "#include \"egui_states.h\"",
        "#define C_GOLDEN_SMALL_NAME \"root.small\"",
        "#define C_GOLDEN_SMALL_TYPE \"i32\"",
        "egui_states_write_i32(writer, (-2147483647 - 1))",
        "egui_states_write_i64(writer, (-INT64_C(9223372036854775807) - 1))",
        "egui_states_write_u64(writer, UINT64_C(18446744073709551615))",
        "egui_states_write_f32(writer, c_golden_f32_from_bits(0x80000000u) /* -0.0 */)",
        "egui_states_write_f64(writer, c_golden_f64_from_bits(UINT64_C(0x7ff8000000000000)) /* NaN */)",
        // Every byte survives, including NUL; `?` is escaped against trigraphs.
        "egui_states_write_string(writer, \"a\\\"b\\?\\000\\305\\276\", 7)",
        "#define C_GOLDEN_OUTER_TYPE \"Outer\"",
        "egui_states_server_add_value(server, C_GOLDEN_OUTER_NAME, C_GOLDEN_OUTER_TYPE, writer, EGUI_STATES_QUEUE, NULL)",
        "egui_states_server_add_value(server, C_GOLDEN_SMALL_NAME, C_GOLDEN_SMALL_TYPE, writer, 0, NULL)",
        // Enums are written as their variant index, here High at index 2.
        "egui_states_write_enum(writer, 2)",
        "#define C_GOLDEN_LIST_TYPE \"Vec<Sparse>\"",
        "egui_states_server_add_static(server, C_GOLDEN_LIST_NAME, C_GOLDEN_LIST_TYPE, writer, NULL)",
        "#define C_GOLDEN_LOOKUP_KEY_TYPE \"String\"",
        "#define C_GOLDEN_LOOKUP_VALUE_TYPE \"(u8, [u16; 2])\"",
        "egui_states_server_add_map(server, C_GOLDEN_LOOKUP_NAME, C_GOLDEN_LOOKUP_KEY_TYPE, C_GOLDEN_LOOKUP_VALUE_TYPE, NULL)",
        "#define C_GOLDEN_ITEMS_TYPE \"Option<u32>\"",
        "#define C_GOLDEN_PULSE_TYPE \"()\"",
        "#define C_GOLDEN_TAKE_TYPE \"(u8,)\"",
        "#define C_GOLDEN_SAMPLES_DATA_TYPE EGUI_STATES_DATA_F32",
        "egui_states_server_add_data(server, C_GOLDEN_SAMPLES_NAME, C_GOLDEN_SAMPLES_DATA_TYPE, NULL)",
        "egui_states_server_add_image(server, C_GOLDEN_IMAGE_NAME, NULL)",
        "#define C_GOLDEN_NESTED_VALUE_NAME \"root.nested.value\"",
        "typedef enum c_golden_sparse_t {",
        "    C_GOLDEN_SPARSE_LOW = -7,",
        "    C_GOLDEN_SPARSE_HIGH = 201,",
        "static inline bool c_golden_sparse_to_index(c_golden_sparse_t value, uint32_t *index)",
        "static inline bool c_golden_sparse_from_index(uint32_t index, c_golden_sparse_t *value)",
        "static inline egui_states_status_t c_golden_register(egui_states_server_t *server)",
    ] {
        assert!(
            header.contains(expected),
            "missing {expected:?} in\n{header}"
        );
    }

    // Map entries keep the canonical key order of the client.
    let labels = header.find("C_GOLDEN_LABELS_TYPE").unwrap();
    let a = header[labels..].find("\"a\", 1").unwrap();
    let b = header[labels..].find("\"b\", 1").unwrap();
    assert!(a < b);

    // Each definition follows the types it uses.
    let position = |text: &str| header.find(text).unwrap_or_else(|| panic!("{text}"));
    let outer = position("\"struct Outer { inner: Option<Inner>, choice: Sparse }\"");
    assert!(position("\"enum Sparse { Low = -7, Mid = 3, High = 201 }\"") < outer);
    assert!(position("\"struct Inner { flag: bool, level: u16 }\"") < outer);

    let client = ClientBuilder::<Golden>::new().get_version_hash();
    assert!(header.contains(&format!("#define C_GOLDEN_VERSION_HASH UINT64_C({client})")));
    std::fs::remove_dir_all(dir).unwrap();
}

fn modified(path: &Path) -> std::time::SystemTime {
    path.metadata().unwrap().modified().unwrap()
}

#[test]
fn generation_is_deterministic_and_skips_unchanged_files() {
    let (first_dir, first) = generate::<Golden>("first");
    let (second_dir, second) = generate::<Golden>("second");
    assert_eq!(first, second);

    let file = first_dir.join("c_golden.h");
    let before = modified(&file);
    std::thread::sleep(std::time::Duration::from_millis(20));
    generate_c::<Golden>(&first_dir).unwrap();
    assert_eq!(modified(&file), before, "rewrote an unchanged header");

    std::fs::remove_dir_all(first_dir).unwrap();
    std::fs::remove_dir_all(second_dir).unwrap();
}

struct Colliding;

impl State for Colliding {
    const NAME: &'static str = "Colliding";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: MapState<u8, u8> = c.map("x");
        let _: Value<u8> = c.value("x_key", 0);
        Self
    }
}

#[test]
#[should_panic(expected = "COLLIDING_X_KEY_TYPE of root.x_key conflicts")]
fn names_that_collide_in_c_are_rejected() {
    let _ = generate::<Colliding>("colliding");
}

struct EguiStates;

impl State for EguiStates {
    const NAME: &'static str = "EguiStates";

    fn new(_: &mut impl StatesCreator) -> Self {
        Self
    }
}

#[test]
#[should_panic(expected = "would generate egui_states.h")]
fn a_state_named_like_the_library_header_is_rejected() {
    let _ = generate::<EguiStates>("library_name");
}

/// Reads the C string literals of `#define <name> "<text>"` lines and of the
/// type definitions.
fn string_literals(header: &str) -> (Vec<(String, String)>, Vec<String>) {
    fn unescape(literal: &str) -> String {
        let mut bytes = Vec::new();
        let mut chars = literal.trim_matches('"').bytes().peekable();
        while let Some(byte) = chars.next() {
            if byte != b'\\' {
                bytes.push(byte);
                continue;
            }
            match chars.next().unwrap() {
                digit @ b'0'..=b'7' => {
                    let mut value = u32::from(digit - b'0');
                    for _ in 0..2 {
                        value = value * 8 + u32::from(chars.next().unwrap() - b'0');
                    }
                    bytes.push(value as u8);
                }
                other => bytes.push(other),
            }
        }
        String::from_utf8(bytes).unwrap()
    }

    let mut types = Vec::new();
    let mut definitions = Vec::new();
    for line in header.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#define ") {
            if let Some((name, literal)) = rest.split_once(' ')
                && name.ends_with("_TYPE")
                && literal.starts_with('"')
            {
                types.push((name.to_owned(), unescape(literal)));
            }
        } else if line.starts_with("\"struct ") || line.starts_with("\"enum ") {
            definitions.push(unescape(line.trim_end_matches(',')));
        }
    }
    (types, definitions)
}

/// The emitted strings are what the C ABI parses at runtime, so each must
/// parse and hash like the Rust type it describes.
#[test]
fn emitted_type_strings_parse_and_match_the_rust_types() {
    use egui_states::Typed;
    use egui_states::c_abi::CServer;

    let (dir, header) = generate::<Golden>("parsed");
    let (types, definitions) = string_literals(&header);
    assert_eq!(definitions.len(), 3);
    let server = CServer::new(None);
    for definition in &definitions {
        server.define_type(definition).unwrap();
    }
    let types: HashMap<_, _> = types.into_iter().collect();
    for (name, rust_type) in [
        ("C_GOLDEN_OUTER_TYPE", Outer::get_type()),
        ("C_GOLDEN_LIST_TYPE", <Vec<Sparse>>::get_type()),
        ("C_GOLDEN_LABELS_TYPE", <HashMap<String, u8>>::get_type()),
        ("C_GOLDEN_LOOKUP_VALUE_TYPE", <(u8, [u16; 2])>::get_type()),
        ("C_GOLDEN_ITEMS_TYPE", <Option<u32>>::get_type()),
        ("C_GOLDEN_PULSE_TYPE", <()>::get_type()),
        ("C_GOLDEN_TAKE_TYPE", <(u8,)>::get_type()),
        ("C_GOLDEN_TEXT_TYPE", String::get_type()),
    ] {
        let text = &types[name];
        assert_eq!(
            server.type_hash(text).unwrap(),
            rust_type.get_hash(),
            "{name}"
        );
    }
    for text in types.values() {
        server.type_hash(text).unwrap();
    }

    let id = |name: &str| {
        let marker = format!("#define {name} UINT64_C(");
        let start = header.find(&marker).unwrap() + marker.len();
        header[start..]
            .split(')')
            .next()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    assert_eq!(
        id("C_GOLDEN_NESTED_VALUE_ID"),
        egui_states::c_abi::state_id("root.nested.value")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
