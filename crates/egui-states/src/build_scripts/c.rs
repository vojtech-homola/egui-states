use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;

use crate::InitValue;
use crate::State;
use crate::build_scripts::scripts;
use crate::build_scripts::states_creator_build::StateType;
use crate::data_transport::DataType;
use crate::hashing::generate_value_id;
use crate::typed::{ObjectType, type_definition, type_expression};

/// Converts a Rust type name such as `CounterState` or `HTTPServer` to
/// `counter_state` or `http_server`.
fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut output = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let previous = index.checked_sub(1).map(|i| chars[i]);
            let next = chars.get(index + 1);
            let starts_word = previous.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || (previous.is_some_and(char::is_uppercase)
                    && next.is_some_and(|n| n.is_lowercase()));
            if starts_word && !output.ends_with('_') {
                output.push('_');
            }
            output.extend(c.to_lowercase());
        } else {
            output.push(c);
        }
    }
    output
}

/// Spells `text` with ASCII letters, digits, and underscores only.
fn identifier(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn upper(text: &str) -> String {
    identifier(text).to_ascii_uppercase()
}

/// A C string literal holding exactly the bytes of `text`.
///
/// Non-printable and non-ASCII bytes use three-digit octal escapes, which
/// cannot absorb following digits, and `?` is escaped to avoid trigraphs.
fn c_string(text: &str) -> String {
    let mut output = String::from("\"");
    for byte in text.bytes() {
        match byte {
            b'"' => output.push_str("\\\""),
            b'\\' => output.push_str("\\\\"),
            b'?' => output.push_str("\\?"),
            0x20..=0x7e => output.push(byte as char),
            _ => write!(output, "\\{byte:03o}").unwrap(),
        }
    }
    output.push('"');
    output
}

fn data_type_constant(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::U8 => "EGUI_STATES_DATA_U8",
        DataType::U16 => "EGUI_STATES_DATA_U16",
        DataType::U32 => "EGUI_STATES_DATA_U32",
        DataType::U64 => "EGUI_STATES_DATA_U64",
        DataType::I8 => "EGUI_STATES_DATA_I8",
        DataType::I16 => "EGUI_STATES_DATA_I16",
        DataType::I32 => "EGUI_STATES_DATA_I32",
        DataType::I64 => "EGUI_STATES_DATA_I64",
        DataType::F32 => "EGUI_STATES_DATA_F32",
        DataType::F64 => "EGUI_STATES_DATA_F64",
    }
}

/// Every emitted macro, type, and function name, so generated names that
/// collapse to the same C identifier are reported instead of redefined.
struct Names(HashSet<String>);

impl Names {
    fn claim(&mut self, name: String, origin: &str) -> String {
        if !self.0.insert(name.clone()) {
            panic!("C identifier {name} of {origin} conflicts with another generated name");
        }
        name
    }
}

/// One registered state with its full dotted name.
struct Entry<'a> {
    full_name: String,
    state: &'a StateType,
}

fn collect_entries<'a>(parent: &str, states: &'a [StateType], entries: &mut Vec<Entry<'a>>) {
    for state in states {
        let full_name = format!("{parent}.{}", state.name());
        match state {
            StateType::SubState(_, _, children) => collect_entries(&full_name, children, entries),
            _ => entries.push(Entry { full_name, state }),
        }
    }
}

/// Appends the definitions of named types used by `object_type`, each after
/// the types it uses.
fn collect_definitions(object_type: &ObjectType, definitions: &mut Vec<ObjectType>) {
    match object_type {
        ObjectType::Struct(_, fields) => {
            for (_, field) in fields {
                collect_definitions(field, definitions);
            }
            if !definitions.contains(object_type) {
                definitions.push(object_type.clone());
            }
        }
        ObjectType::Enum(..) => {
            if !definitions.contains(object_type) {
                definitions.push(object_type.clone());
            }
        }
        ObjectType::Tuple(elements) => {
            for element in elements {
                collect_definitions(element, definitions);
            }
        }
        ObjectType::List(_, element) | ObjectType::Vec(element) | ObjectType::Option(element) => {
            collect_definitions(element, definitions)
        }
        ObjectType::Map(key, value) => {
            collect_definitions(key, definitions);
            collect_definitions(value, definitions);
        }
        _ => {}
    }
}

fn state_types(state: &StateType) -> Vec<&ObjectType> {
    match state {
        StateType::Value(_, object_type, ..)
        | StateType::ValueTake(_, object_type)
        | StateType::Static(_, object_type, _)
        | StateType::ValueVec(_, object_type)
        | StateType::Signal(_, object_type, _) => vec![object_type],
        StateType::ValueMap(_, key, value) => vec![key, value],
        _ => Vec::new(),
    }
}

struct Generator {
    prefix: String,
    upper: String,
    uses_f32: bool,
    uses_f64: bool,
}

impl Generator {
    fn check(&self, call: String) -> String {
        format!("    if ((status = {call}) != EGUI_STATES_OK) goto done;\n")
    }

    fn write(&self, function: &str, arguments: &str) -> String {
        self.check(format!("egui_states_write_{function}(writer{arguments})"))
    }

    /// Emits the writer calls that encode `init` as `object_type`.
    fn init_value(&mut self, init: &InitValue, object_type: &ObjectType, output: &mut String) {
        let line = match (init, object_type) {
            (InitValue::U8(v), ObjectType::U8) => self.write("u8", &format!(", {v}")),
            (InitValue::U16(v), ObjectType::U16) => self.write("u16", &format!(", {v}")),
            (InitValue::U32(v), ObjectType::U32) => self.write("u32", &format!(", {v}u")),
            (InitValue::U64(v), ObjectType::U64) => self.write("u64", &format!(", UINT64_C({v})")),
            (InitValue::I8(v), ObjectType::I8) => self.write("i8", &format!(", {v}")),
            (InitValue::I16(v), ObjectType::I16) => self.write("i16", &format!(", {v}")),
            // The minimums are written as expressions: a negated literal that
            // does not fit the signed type is unsigned for some compilers.
            (InitValue::I32(v), ObjectType::I32) if *v == i32::MIN => {
                self.write("i32", ", (-2147483647 - 1)")
            }
            (InitValue::I32(v), ObjectType::I32) => self.write("i32", &format!(", {v}")),
            (InitValue::I64(v), ObjectType::I64) if *v == i64::MIN => {
                self.write("i64", ", (-INT64_C(9223372036854775807) - 1)")
            }
            (InitValue::I64(v), ObjectType::I64) => self.write("i64", &format!(", INT64_C({v})")),
            // Floats are written by their bits, which keeps NaN, infinities,
            // and negative zero exact.
            (InitValue::F32(v), ObjectType::F32) => {
                self.uses_f32 = true;
                let bits = v.to_bits();
                self.write(
                    "f32",
                    &format!(", {}_f32_from_bits(0x{bits:08x}u) /* {v:?} */", self.prefix),
                )
            }
            (InitValue::F64(v), ObjectType::F64) => {
                self.uses_f64 = true;
                let bits = v.to_bits();
                self.write(
                    "f64",
                    &format!(
                        ", {}_f64_from_bits(UINT64_C(0x{bits:016x})) /* {v:?} */",
                        self.prefix
                    ),
                )
            }
            (InitValue::Bool(v), ObjectType::Bool) => {
                self.write("bool", &format!(", {}", u8::from(*v)))
            }
            (InitValue::String(text), ObjectType::String) => {
                self.write("string", &format!(", {}, {}", c_string(text), text.len()))
            }
            (InitValue::Enum(variant), ObjectType::Enum(name, variants)) => {
                let index = variants
                    .iter()
                    .position(|(candidate, _)| candidate == variant)
                    .unwrap_or_else(|| panic!("Enum {name} has no variant {variant}"));
                self.write("enum", &format!(", {index}"))
                    + &format!("    /* {name}::{variant} */\n")
            }
            (InitValue::Option(None), ObjectType::Option(_)) => self.write("none", ""),
            (InitValue::Option(Some(inner)), ObjectType::Option(inner_type)) => {
                output.push_str(&self.write("some", ""));
                self.init_value(inner, inner_type, output);
                return;
            }
            (InitValue::Struct(_, fields), ObjectType::Struct(_, field_types)) => {
                for ((_, field), (_, field_type)) in fields.iter().zip(field_types) {
                    self.init_value(field, field_type, output);
                }
                return;
            }
            // `()` describes its initial value as an empty tuple.
            (InitValue::Tuple(items), ObjectType::Empty) if items.is_empty() => return,
            (InitValue::Tuple(items), ObjectType::Tuple(types)) => {
                for (item, item_type) in items.iter().zip(types) {
                    self.init_value(item, item_type, output);
                }
                return;
            }
            (InitValue::List(items), ObjectType::List(_, item_type)) => {
                for item in items {
                    self.init_value(item, item_type, output);
                }
                return;
            }
            (InitValue::Vec(items), ObjectType::Vec(item_type)) => {
                output.push_str(&self.write("len", &format!(", {}", items.len())));
                for item in items {
                    self.init_value(item, item_type, output);
                }
                return;
            }
            (InitValue::Map(entries), ObjectType::Map(key_type, value_type)) => {
                output.push_str(&self.write("len", &format!(", {}", entries.len())));
                for (key, value) in entries {
                    self.init_value(key, key_type, output);
                    self.init_value(value, value_type, output);
                }
                return;
            }
            (init, object_type) => {
                panic!("initial value {init:?} does not match type {object_type:?}")
            }
        };
        output.push_str(&line);
    }
}

fn render_c<S: State>() -> (String, String) {
    let (states, version_hash, _) = scripts::parse_states::<S>();
    scripts::validate_states(&states);
    let StateType::SubState(_, _, children) = &states else {
        panic!("Root state must be a SubState");
    };

    let prefix = snake_case(S::NAME);
    if prefix == "egui_states" {
        panic!(
            "State {} would generate egui_states.h, the library header",
            S::NAME
        );
    }
    let mut generator = Generator {
        upper: upper(&prefix),
        prefix: identifier(&prefix),
        uses_f32: false,
        uses_f64: false,
    };
    let p = generator.prefix.clone();
    let pu = generator.upper.clone();
    let mut names = Names(HashSet::new());
    let guard = names.claim(format!("{pu}_H"), "the include guard");

    let mut entries = Vec::new();
    collect_entries("root", children, &mut entries);

    // Rejects conflicting definitions of one type name, as the other generators do.
    let mut values = Vec::new();
    scripts::states_into_values_list(&states, &mut values);
    scripts::get_all_enums_struct(&values);
    let mut definitions = Vec::new();
    for entry in &entries {
        for object_type in state_types(entry.state) {
            collect_definitions(object_type, &mut definitions);
        }
    }

    let mut constants = String::new();
    let version = names.claim(format!("{pu}_VERSION_HASH"), "the layout hash");
    writeln!(
        constants,
        "/* Layout hash of the state tree; pass it to egui_states_server_new to\n * reject clients built from a different state definition. */\n#define {version} UINT64_C({version_hash})\n"
    )
    .unwrap();

    let mut registrations = String::new();
    for entry in &entries {
        let full_name = &entry.full_name;
        let path = upper(full_name.strip_prefix("root.").unwrap_or(full_name));
        let macro_name = |suffix: &str| format!("{pu}_{path}_{suffix}");
        let id = names.claim(macro_name("ID"), full_name);
        let name = names.claim(macro_name("NAME"), full_name);
        let kind = match entry.state {
            StateType::Value(..) => "Value",
            StateType::ValueTake(..) => "ValueTake",
            StateType::Static(..) => "Static",
            StateType::Signal(..) => "Signal",
            StateType::ValueVec(..) => "VecState",
            StateType::ValueMap(..) => "MapState",
            StateType::Image(..) => "Image",
            StateType::ImageMulti(..) => "ImageMulti",
            StateType::Data(..) => "Data",
            StateType::DataTake(..) => "DataTake",
            StateType::DataMulti(..) => "DataMulti",
            StateType::DataMultiTake(..) => "DataMultiTake",
            StateType::SubState(..) => unreachable!("substates are flattened"),
        };
        writeln!(constants, "/* {kind} {} */", full_name.replace("*/", "* /")).unwrap();
        writeln!(
            constants,
            "#define {id} UINT64_C({})",
            generate_value_id(full_name)
        )
        .unwrap();
        writeln!(constants, "#define {name} {}", c_string(full_name)).unwrap();

        let mut type_macro = |suffix: &str, object_type: &ObjectType| {
            let type_name = names.claim(macro_name(suffix), full_name);
            writeln!(
                constants,
                "#define {type_name} {}",
                c_string(&type_expression(object_type))
            )
            .unwrap();
            type_name
        };
        let queue = |queue: bool| if queue { "EGUI_STATES_QUEUE" } else { "0" };
        match entry.state {
            StateType::Value(_, object_type, init, is_queue) => {
                let type_name = type_macro("TYPE", object_type);
                registrations.push_str("    egui_states_writer_clear(writer);\n");
                generator.init_value(init, object_type, &mut registrations);
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_value(server, {name}, {type_name}, writer, {}, NULL)",
                    queue(*is_queue)
                )));
            }
            StateType::Static(_, object_type, init) => {
                let type_name = type_macro("TYPE", object_type);
                registrations.push_str("    egui_states_writer_clear(writer);\n");
                generator.init_value(init, object_type, &mut registrations);
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_static(server, {name}, {type_name}, writer, NULL)"
                )));
            }
            StateType::ValueTake(_, object_type) => {
                let type_name = type_macro("TYPE", object_type);
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_value_take(server, {name}, {type_name}, NULL)"
                )));
            }
            StateType::Signal(_, object_type, is_queue) => {
                let type_name = type_macro("TYPE", object_type);
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_signal(server, {name}, {type_name}, {}, NULL)",
                    queue(*is_queue)
                )));
            }
            StateType::ValueVec(_, object_type) => {
                let type_name = type_macro("TYPE", object_type);
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_vec(server, {name}, {type_name}, NULL)"
                )));
            }
            StateType::ValueMap(_, key, value) => {
                let key_name = type_macro("KEY_TYPE", key);
                let value_name = type_macro("VALUE_TYPE", value);
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_map(server, {name}, {key_name}, {value_name}, NULL)"
                )));
            }
            StateType::Image(_) => registrations.push_str(&generator.check(format!(
                "egui_states_server_add_image(server, {name}, NULL)"
            ))),
            StateType::ImageMulti(_) => registrations.push_str(&generator.check(format!(
                "egui_states_server_add_image_multi(server, {name}, NULL)"
            ))),
            StateType::Data(_, data_type)
            | StateType::DataTake(_, data_type)
            | StateType::DataMulti(_, data_type)
            | StateType::DataMultiTake(_, data_type) => {
                let data_name = names.claim(macro_name("DATA_TYPE"), full_name);
                writeln!(
                    constants,
                    "#define {data_name} {}",
                    data_type_constant(data_type)
                )
                .unwrap();
                let function = match entry.state {
                    StateType::Data(..) => "data",
                    StateType::DataTake(..) => "data_take",
                    StateType::DataMulti(..) => "data_multi",
                    _ => "data_multi_take",
                };
                registrations.push_str(&generator.check(format!(
                    "egui_states_server_add_{function}(server, {name}, {data_name}, NULL)"
                )));
            }
            StateType::SubState(..) => unreachable!("substates are flattened"),
        }
        constants.push('\n');
    }

    let mut enums = String::new();
    for definition in &definitions {
        let ObjectType::Enum(enum_name, variants) = definition else {
            continue;
        };
        if variants.is_empty() {
            continue;
        }
        let snake = identifier(&snake_case(enum_name));
        let type_name = names.claim(format!("{p}_{snake}_t"), enum_name);
        let constant =
            |variant: &str| format!("{pu}_{}_{}", upper(&snake), upper(&snake_case(variant)));
        writeln!(
            enums,
            "/* Enum {enum_name}. The wire format carries the variant index, not the value. */\ntypedef enum {type_name} {{"
        )
        .unwrap();
        for (variant, value) in variants {
            let name = names.claim(constant(variant), enum_name);
            let value = if *value == i32::MIN {
                "(-2147483647 - 1)".to_owned()
            } else {
                value.to_string()
            };
            writeln!(enums, "    {name} = {value},").unwrap();
        }
        writeln!(enums, "}} {type_name};\n").unwrap();

        let to_index = names.claim(format!("{p}_{snake}_to_index"), enum_name);
        writeln!(
            enums,
            "/* Converts a {enum_name} value to its variant index; false for unknown values. */\nstatic inline bool {to_index}({type_name} value, uint32_t *index) {{\n    switch (value) {{"
        )
        .unwrap();
        for (index, (variant, _)) in variants.iter().enumerate() {
            writeln!(
                enums,
                "    case {}:\n        *index = {index}u;\n        return true;",
                constant(variant)
            )
            .unwrap();
        }
        enums.push_str("    default:\n        return false;\n    }\n}\n\n");

        let from_index = names.claim(format!("{p}_{snake}_from_index"), enum_name);
        writeln!(
            enums,
            "/* Converts a variant index to its {enum_name} value; false for unknown indices. */\nstatic inline bool {from_index}(uint32_t index, {type_name} *value) {{\n    switch (index) {{"
        )
        .unwrap();
        for (index, (variant, _)) in variants.iter().enumerate() {
            writeln!(
                enums,
                "    case {index}u:\n        *value = {};\n        return true;",
                constant(variant)
            )
            .unwrap();
        }
        enums.push_str("    default:\n        return false;\n    }\n}\n\n");
    }

    let mut helpers = String::new();
    if generator.uses_f32 {
        let name = names.claim(format!("{p}_f32_from_bits"), "a float helper");
        writeln!(
            helpers,
            "static inline float {name}(uint32_t bits) {{\n    float value;\n    memcpy(&value, &bits, sizeof value);\n    return value;\n}}\n"
        )
        .unwrap();
    }
    if generator.uses_f64 {
        let name = names.claim(format!("{p}_f64_from_bits"), "a float helper");
        writeln!(
            helpers,
            "static inline double {name}(uint64_t bits) {{\n    double value;\n    memcpy(&value, &bits, sizeof value);\n    return value;\n}}\n"
        )
        .unwrap();
    }

    let register = names.claim(format!("{p}_register"), "the register function");
    let mut function = format!(
        "/* Defines the types and registers every state of {state} on `server` with its\n * initial value. Call egui_states_server_finalize next. On failure, it returns\n * the failing status; egui_states_last_error() describes it. */\nstatic inline egui_states_status_t {register}(egui_states_server_t *server) {{\n    egui_states_status_t status = EGUI_STATES_OK;\n    egui_states_writer_t *writer = NULL;\n",
        state = S::NAME
    );
    if !definitions.is_empty() {
        writeln!(
            function,
            "    static const char *const definitions[{}] = {{",
            definitions.len()
        )
        .unwrap();
        for definition in &definitions {
            let text = type_definition(definition).expect("definitions are named types");
            writeln!(function, "        {},", c_string(&text)).unwrap();
        }
        function.push_str("    };\n    size_t index;\n\n");
        writeln!(
            function,
            "    for (index = 0; index < {}; ++index) {{\n        status = egui_states_server_define_type(server, definitions[index]);\n        if (status != EGUI_STATES_OK) {{\n            return status;\n        }}\n    }}",
            definitions.len()
        )
        .unwrap();
    }
    function.push_str(
        "\n    status = egui_states_writer_new(&writer);\n    if (status != EGUI_STATES_OK) {\n        return status;\n    }\n\n",
    );
    function.push_str(&registrations);
    function.push_str("\ndone:\n    egui_states_writer_free(writer);\n    return status;\n}\n");

    let header = format!(
        "/* Generated by build.rs, do not edit.\n *\n * C registration of the {state} state tree for the egui-states C ABI.\n * Include it after adding the directory of egui_states.h to the include path. */\n\n#ifndef {guard}\n#define {guard}\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <string.h>\n\n#include \"egui_states.h\"\n\n{constants}{enums}{helpers}{function}\n#endif /* {guard} */\n",
        state = S::NAME
    );
    (format!("{p}.h"), header)
}

/// Generates a C registration header from `S` for the experimental C ABI.
///
/// Available with the `build_scripts` and `c_abi` features.
///
/// The header is written to `directory` as `<prefix>.h`, where the prefix is
/// `S::NAME` in snake case (`CounterState` becomes `counter_state.h`). It
/// needs `egui_states.h` from the `egui_states_c` crate and defines:
///
/// - `<PREFIX>_VERSION_HASH`, the layout hash for the handshake;
/// - for each state, `<PREFIX>_<PATH>_ID` and `_NAME`, plus its type string
///   in `_TYPE` (`_KEY_TYPE` and `_VALUE_TYPE` for maps, `_DATA_TYPE` for
///   numeric buffers);
/// - a C enum per protocol enum, with conversions to and from the variant
///   index that the wire format carries;
/// - `<prefix>_register(server)`, which defines every struct and enum and
///   registers every state with its initial value.
///
/// The output directory is created if needed, and an unchanged file is not
/// rewritten.
///
/// # Errors
///
/// Returns an error if the output directory or file cannot be created or
/// written.
///
/// # Panics
///
/// Panics if a type is defined inconsistently in different locations, if two
/// generated C names collide, or if the prefix would be `egui_states`.
pub fn generate_c<S: State>(directory: impl AsRef<Path>) -> Result<(), String> {
    let (file_name, header) = render_c::<S>();
    scripts::write_generated_files(directory.as_ref(), [(file_name.as_str(), header.as_str())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_c_identifiers() {
        assert_eq!(snake_case("CounterState"), "counter_state");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("State2D"), "state2_d");
        assert_eq!(snake_case("lower"), "lower");
        assert_eq!(upper("nested.ünï"), "NESTED__N_");
    }

    #[test]
    fn strings_keep_their_bytes() {
        assert_eq!(c_string("plain"), "\"plain\"");
        assert_eq!(c_string("a\"b\\c?"), "\"a\\\"b\\\\c\\?\"");
        assert_eq!(c_string("\0x\n"), "\"\\000x\\012\"");
        assert_eq!(c_string("ž"), "\"\\305\\276\"");
    }
}
