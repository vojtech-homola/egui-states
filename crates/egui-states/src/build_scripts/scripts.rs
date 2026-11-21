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

fn collect_state_definitions<'a>(
    state_class: &'static str,
    states: &'a [StateType],
    root: bool,
    definitions: &mut HashMap<&'static str, (bool, &'a [StateType])>,
) {
    if matches!(state_class, "StatesServer" | "enums" | "structs" | "s") {
        panic!("State {state_class} conflicts with a generated symbol");
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
            collect_state_definitions(class, children, false, definitions);
        }
    }
}

pub(crate) fn validate_states(states: &StateType) {
    let StateType::SubState(_, root_name, children, _) = states else {
        panic!("Root state must be a SubState");
    };

    let mut definitions = HashMap::new();
    collect_state_definitions(root_name, children, true, &mut definitions);
}

pub(crate) fn write_generated_files<const N: usize>(
    directory: &Path,
    files: [(&str, &str); N],
) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Failed to create generated output directory {}: {error}",
            directory.display()
        )
    })?;

    for (name, output) in files {
        let path = directory.join(name);
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
    fn collect(&mut self, type_info: &ObjectType) {
        match type_info {
            ObjectType::Enum(name, variants) => {
                if self.enums.get(name).is_some_and(|known| known != variants)
                    || self.data_enums.contains_key(name)
                {
                    panic!("Enum {name} defined multiple times with different variants");
                }
                self.enums.insert(name.clone(), variants.clone());
            }
            ObjectType::DataEnum(name, variants) => {
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
                                self.collect(field_type);
                            }
                        }
                        VariantFields::Tuple(fields) => {
                            for field_type in fields {
                                self.collect(field_type);
                            }
                        }
                    }
                }
            }
            ObjectType::Struct(name, fields) => {
                if self.structs.get(name).is_some_and(|known| known != fields) {
                    panic!("Struct {name} defined multiple times with different fields");
                }
                self.structs.insert(name.clone(), fields.clone());
                for (_, field_type) in fields {
                    self.collect(field_type);
                }
            }
            ObjectType::Tuple(elements) => {
                for element in elements {
                    self.collect(element);
                }
            }
            ObjectType::List(_, element)
            | ObjectType::Option(element)
            | ObjectType::Vec(element) => self.collect(element),
            ObjectType::Map(key_type, value_type) => {
                self.collect(key_type);
                self.collect(value_type);
            }
            _ => { /* ignore basic types */ }
        }
    }
}

pub(crate) fn get_custom_types(values: &[StateType]) -> CustomTypes {
    let mut types = CustomTypes::default();

    for value in values {
        match value {
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
            | StateType::ImageMulti(_, Some(info), _) => types.collect(info),
            StateType::ValueMap(_, key_info, value_info) => {
                types.collect(key_info);
                types.collect(value_info);
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
