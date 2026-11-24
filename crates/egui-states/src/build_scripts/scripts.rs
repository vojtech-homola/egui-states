use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use crate::State;
use crate::build_scripts::states_creator_build::{RustDerives, StateType, StatesCreatorBuild};
use crate::typed::{ObjectType, VariantFields};

/// Enum name to its `(variant, value)` pairs.
pub(crate) type Enums = BTreeMap<String, Vec<(String, i32)>>;
/// Struct name to its `(field, type)` pairs.
pub(crate) type Structs = BTreeMap<String, Vec<(String, ObjectType)>>;
/// Enum with data name to its variants.
pub(crate) type DataEnums = BTreeMap<String, Vec<(String, VariantFields)>>;

pub(crate) fn parse_states<S: State>() -> (StateType, u64, RustDerives) {
    let mut creator = StatesCreatorBuild::new("root");
    let _ = S::new(&mut creator);
    let version_hash = creator.get_version_hash();
    let (states, rust_derives) = creator.into_parts();
    (
        StateType::SubState("root".to_string(), S::NAME, states, false),
        version_hash,
        rust_derives,
    )
}

/// Python's hard keywords. Soft keywords (`match`, `case`, `type`, `_`) are valid
/// identifiers and need no escaping.
pub(crate) const PYTHON_KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// The names the generated `enums.py` binds besides the enums: the `structs`
/// module and the private aliases of what it imports. An enum or a variant
/// with one of these names would replace it for the code that follows.
pub(crate) const ENUMS_MODULE_NAMES: [&str; 5] =
    ["structs", "_dataclass", "_final", "_IntEnum", "_s"];

/// The names the generated `structs.py` binds besides the structs.
const STRUCTS_MODULE_NAMES: [&str; 3] = ["enums", "dataclass", "s"];

/// The names the generated state module binds besides the state groups, in
/// either language: the server and the modules it refers to.
const STATES_MODULE_NAMES: [&str; 4] = ["StatesServer", "enums", "structs", "s"];

/// The further names the generated `__init__.py` binds or uses bare: NumPy,
/// the one builtin class it names, and the private aliases of what it
/// imports. Its imports are aliased so that a state group may be called
/// `Sequence` or `ClientPolicy`.
const PYTHON_STATES_MODULE_NAMES: [&str; 9] = [
    "np",
    "Exception",
    "_Callable",
    "_Sequence",
    "_PathLike",
    "_ClientPolicy",
    "_StatesBase",
    "_StateServerBase",
    "_TransferProfile",
];

/// The language the bindings are generated for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Python,
    Rust,
}

/// Rejects the name of a generated class that Python cannot define, or that
/// would replace one of the `reserved` names of its module. `kind` is
/// `State`, `Enum` or `Struct`, for the message.
///
/// Fields, variants and state names are escaped instead, because the wire
/// keeps their original names; a class name exists only in the bindings. A
/// name that is only a Python keyword, such as `None`, is a valid Rust name,
/// so the Rust bindings keep it.
fn check_class_name(kind: &str, name: &str, reserved: &[&str], target: Target) {
    if name.starts_with("r#") {
        panic!("{kind} {name} is named with a raw identifier, which has no Python spelling");
    }
    if target == Target::Python && PYTHON_KEYWORDS.contains(&name) {
        panic!("{kind} {name} is named like a Python keyword");
    }
    if reserved.contains(&name) {
        panic!("{kind} {name} conflicts with a generated symbol");
    }
}

fn collect_state_definitions<'a>(
    state_class: &'static str,
    states: &'a [StateType],
    root: bool,
    definitions: &mut HashMap<&'static str, (bool, &'a [StateType])>,
    target: Target,
) {
    check_class_name("State", state_class, &STATES_MODULE_NAMES, target);
    if target == Target::Python {
        check_class_name("State", state_class, &PYTHON_STATES_MODULE_NAMES, target);
    }

    let definition = (root, states);
    if let Some(previous) = definitions.get(state_class) {
        if previous != &definition {
            panic!("State {state_class} defined multiple times with different fields");
        }
    } else {
        definitions.insert(state_class, definition);
    }

    for state in states {
        if let StateType::SubState(_, class, children, _) = state {
            collect_state_definitions(class, children, false, definitions, target);
        }
    }
}

pub(crate) fn validate_states(states: &StateType, target: Target) {
    let StateType::SubState(_, root_name, children, _) = states else {
        panic!("Root state must be a SubState");
    };

    let mut definitions = HashMap::new();
    collect_state_definitions(root_name, children, true, &mut definitions, target);
}

/// The line every generated file carries near its top.
const GENERATED_MARKER: &str = "Generated by build.rs, do not edit";

/// Writes the generated `files` into `directory`, leaving a file with the same
/// contents untouched. A file with no contents is not part of this output: a
/// copy an earlier run generated is removed, so that it does not go stale,
/// while a file somebody else wrote stays.
pub(crate) fn write_generated_files<const N: usize>(
    directory: &Path,
    files: [(&str, Option<&str>); N],
) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Failed to create generated output directory {}: {error}",
            directory.display()
        )
    })?;

    for (name, output) in files {
        let path = directory.join(name);
        let Some(output) = output else {
            let generated = fs::read_to_string(&path).is_ok_and(|current| {
                current
                    .lines()
                    .take(3)
                    .any(|l| l.contains(GENERATED_MARKER))
            });
            if generated {
                fs::remove_file(&path)
                    .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
            }
            continue;
        };
        if fs::read_to_string(&path).is_ok_and(|current| current == output) {
            continue;
        }
        fs::write(&path, output)
            .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    }

    Ok(())
}

/// The custom types a state tree uses, by name.
#[derive(Default)]
pub(crate) struct CustomTypes {
    /// Fieldless enums and their `(variant, value)` pairs.
    pub(crate) enums: Enums,
    /// Enums with data and their variants.
    pub(crate) data_enums: DataEnums,
    /// Structs and their `(field, type)` pairs.
    pub(crate) structs: Structs,
}

impl CustomTypes {
    /// Collects the custom types in `type_info`, which the state `state`
    /// uses; `nested` inside another type.
    ///
    /// # Panics
    ///
    /// Panics if a type name is defined twice with different contents or
    /// cannot be generated, or if `type_info` holds the unit type.
    fn collect(&mut self, state: &str, type_info: &ObjectType, nested: bool, target: Target) {
        match type_info {
            // A value without content has no Python spelling as a field or
            // an element, and no stored value.
            ObjectType::Empty if nested => panic!(
                "State {state}: the unit type `()` is not supported inside another type; only a Signal or a ValueTake takes it, as its whole type"
            ),
            ObjectType::Empty => panic!(
                "State {state}: the unit type `()` is supported only as the type of a Signal or a ValueTake"
            ),
            ObjectType::Enum(name, variants) => {
                check_class_name("Enum", name, &ENUMS_MODULE_NAMES, target);
                if self.enums.get(name).is_some_and(|known| known != variants)
                    || self.data_enums.contains_key(name)
                {
                    panic!("Enum {name} defined multiple times with different variants");
                }
                self.enums.insert(name.clone(), variants.clone());
            }
            ObjectType::DataEnum(name, variants) => {
                check_class_name("Enum", name, &ENUMS_MODULE_NAMES, target);
                if self
                    .data_enums
                    .get(name)
                    .is_some_and(|known| known != variants)
                    || self.enums.contains_key(name)
                {
                    panic!("Enum {name} defined multiple times with different variants");
                }
                self.data_enums.insert(name.clone(), variants.clone());
                for (_, fields) in variants {
                    match fields {
                        VariantFields::Unit => {}
                        VariantFields::Named(fields) => {
                            for (_, field_type) in fields {
                                self.collect(state, field_type, true, target);
                            }
                        }
                        VariantFields::Tuple(fields) => {
                            for field_type in fields {
                                self.collect(state, field_type, true, target);
                            }
                        }
                    }
                }
            }
            ObjectType::Struct(name, fields) => {
                check_class_name("Struct", name, &STRUCTS_MODULE_NAMES, target);
                if self.structs.get(name).is_some_and(|known| known != fields) {
                    panic!("Struct {name} defined multiple times with different fields");
                }
                self.structs.insert(name.clone(), fields.clone());
                for (_, field_type) in fields {
                    self.collect(state, field_type, true, target);
                }
            }
            ObjectType::Tuple(elements) => {
                for element in elements {
                    self.collect(state, element, true, target);
                }
            }
            ObjectType::List(_, element)
            | ObjectType::Option(element)
            | ObjectType::Vec(element) => self.collect(state, element, true, target),
            ObjectType::Map(key_type, value_type) => {
                self.collect(state, key_type, true, target);
                self.collect(state, value_type, true, target);
            }
            _ => { /* ignore basic types */ }
        }
    }
}

/// Collects the custom types of the states `values`, checking every type on
/// the way; see [`CustomTypes::collect`] for what it rejects.
pub(crate) fn get_custom_types(values: &[StateType], target: Target) -> CustomTypes {
    let mut types = CustomTypes::default();

    for value in values {
        let state = value.name();
        match value {
            // Without a payload: the one place for the unit type.
            StateType::Signal(_, ObjectType::Empty, _)
            | StateType::ValueTake(_, ObjectType::Empty) => {}
            StateType::Value(_, info, _, _)
            | StateType::Static(_, info, _)
            | StateType::ValueVec(_, info)
            | StateType::Signal(_, info, _)
            | StateType::ValueTake(_, info)
            | StateType::Data(_, _, Some(info), _)
            | StateType::DataTake(_, _, Some(info), _)
            | StateType::DataMulti(_, _, Some(info), _)
            | StateType::DataMultiTake(_, _, Some(info), _)
            | StateType::Download(_, Some(info))
            | StateType::Upload(_, Some(info))
            | StateType::Image(_, Some(info), _)
            | StateType::ImageMulti(_, Some(info), _) => types.collect(state, info, false, target),
            StateType::ValueMap(_, key_info, value_info) => {
                types.collect(state, key_info, false, target);
                types.collect(state, value_info, false, target);
            }
            _ => { /* ignore other types */ }
        }
    }

    types
}

pub(crate) fn states_into_values_list(state: &StateType, list: &mut Vec<StateType>) {
    match state {
        StateType::SubState(_, _, sub_values, _) => {
            for sub_value in sub_values {
                states_into_values_list(sub_value, list);
            }
        }
        _ => {
            list.push(state.clone());
        }
    }
}
