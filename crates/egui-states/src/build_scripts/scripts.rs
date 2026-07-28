use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::State;
use crate::build_scripts::states_creator_build::{StateType, StatesCreatorBuild};
use crate::typed::ObjectType;

pub(crate) fn parse_states<S: State>() -> (StateType, u64) {
    let mut creator = StatesCreatorBuild::new("root");
    let _ = S::new(&mut creator);
    let version_hash = creator.get_version_hash();
    let states = creator.get_states();
    (
        StateType::SubState("root".to_string(), S::NAME, states),
        version_hash,
    )
}

fn collect_state_definitions<'a>(
    state_class: &'static str,
    states: &'a [StateType],
    root: bool,
    definitions: &mut BTreeMap<&'static str, (bool, &'a [StateType])>,
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
        if let StateType::SubState(_, class, children) = state {
            collect_state_definitions(class, children, false, definitions);
        }
    }
}

pub(crate) fn validate_states(states: &StateType) {
    let StateType::SubState(_, root_name, children) = states else {
        panic!("Root state must be a SubState");
    };

    let mut definitions = BTreeMap::new();
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

fn collect_enums(type_info: &ObjectType, enums: &mut BTreeMap<String, Vec<(String, i32)>>) {
    match type_info {
        ObjectType::Enum(name, variants) => {
            if enums.contains_key(name) {
                if enums[name] != *variants {
                    panic!(
                        "Enum {} defined multiple times with different variants",
                        name
                    );
                }
            }

            enums.insert(name.clone(), variants.clone());
        }
        ObjectType::Struct(_, fields) => {
            for (_, field_type) in fields {
                collect_enums(field_type, enums);
            }
        }
        ObjectType::Tuple(elements) => {
            for elem in elements {
                collect_enums(elem, enums);
            }
        }
        ObjectType::List(_, element) => {
            collect_enums(element, enums);
        }
        ObjectType::Option(element) => {
            collect_enums(element, enums);
        }
        ObjectType::Vec(element) => {
            collect_enums(element, enums);
        }
        ObjectType::Map(key_type, value_type) => {
            collect_enums(key_type, enums);
            collect_enums(value_type, enums);
        }
        _ => { /* ignore basic types */ }
    }
}

fn collect_structs(
    type_info: &ObjectType,
    structs: &mut BTreeMap<String, Vec<(String, ObjectType)>>,
) {
    match type_info {
        ObjectType::Struct(name, fields) => {
            if structs.contains_key(name) {
                if structs[name] != *fields {
                    panic!(
                        "Struct {} defined multiple times with different fields",
                        name
                    );
                }
            }

            structs.insert(name.clone(), fields.clone());
            for (_, field_type) in fields {
                collect_structs(field_type, structs);
            }
        }
        ObjectType::Enum(_, variants) => {
            for (_, _) in variants {
                // Enums don't have nested types in this design
            }
        }
        ObjectType::Tuple(elements) => {
            for elem in elements {
                collect_structs(elem, structs);
            }
        }
        ObjectType::List(_, element) => {
            collect_structs(element, structs);
        }
        ObjectType::Option(element) => {
            collect_structs(element, structs);
        }
        ObjectType::Vec(element) => {
            collect_structs(element, structs);
        }
        ObjectType::Map(key_type, value_type) => {
            collect_structs(key_type, structs);
            collect_structs(value_type, structs);
        }
        _ => { /* ignore basic types */ }
    }
}

pub(crate) fn get_all_enums_struct(
    values: &[StateType],
) -> (
    BTreeMap<String, Vec<(String, i32)>>,
    BTreeMap<String, Vec<(String, ObjectType)>>,
) {
    let mut enums = BTreeMap::new();
    let mut structs = BTreeMap::new();

    for value in values {
        match value {
            StateType::Value(_, info, _, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            StateType::Static(_, info, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            StateType::ValueMap(_, key_info, value_info) => {
                collect_enums(key_info, &mut enums);
                collect_enums(value_info, &mut enums);
                collect_structs(key_info, &mut structs);
                collect_structs(value_info, &mut structs);
            }
            StateType::ValueVec(_, elem_info) => {
                collect_enums(elem_info, &mut enums);
                collect_structs(elem_info, &mut structs);
            }
            StateType::Signal(_, info, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            StateType::ValueTake(_, info) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            _ => { /* ignore other types */ }
        }
    }

    (enums, structs)
}

pub(crate) fn states_into_values_list(state: &StateType, list: &mut Vec<StateType>) {
    match state {
        StateType::SubState(_, _, sub_values) => {
            for sub_value in sub_values {
                states_into_values_list(sub_value, list);
            }
        }
        _ => {
            list.push(state.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InitValue;

    fn value(name: &str, typ: ObjectType, init: InitValue, queue: bool) -> StateType {
        StateType::Value(name.to_string(), typ, init, queue)
    }

    fn repeated_shared_state(left: Vec<StateType>, right: Vec<StateType>) -> StateType {
        StateType::SubState(
            "root".to_string(),
            "Root",
            vec![
                StateType::SubState("left".to_string(), "Shared", left),
                StateType::SubState("right".to_string(), "Shared", right),
            ],
        )
    }

    fn assert_incompatible(left: Vec<StateType>, right: Vec<StateType>) {
        let states = repeated_shared_state(left, right);
        let result = std::panic::catch_unwind(|| validate_states(&states));
        assert!(result.is_err(), "Expected incompatible definitions");
    }

    #[test]
    fn repeated_identical_substate_definition_is_valid() {
        let definition = vec![value(
            "count",
            ObjectType::F32,
            InitValue::F32(f32::NAN),
            false,
        )];
        let states = repeated_shared_state(definition.clone(), definition);

        validate_states(&states);
    }

    #[test]
    fn incompatible_substate_definitions_are_rejected() {
        let base = value("count", ObjectType::I32, InitValue::I32(0), false);

        let incompatible = [
            (
                vec![base.clone()],
                vec![value("other", ObjectType::I32, InitValue::I32(0), false)],
            ),
            (
                vec![base.clone()],
                vec![value(
                    "count",
                    ObjectType::String,
                    InitValue::String(String::new()),
                    false,
                )],
            ),
            (
                vec![base.clone()],
                vec![value("count", ObjectType::I32, InitValue::I32(1), false)],
            ),
            (
                vec![base.clone()],
                vec![value("count", ObjectType::I32, InitValue::I32(0), true)],
            ),
            (
                vec![base.clone()],
                vec![StateType::Static(
                    "count".to_string(),
                    ObjectType::I32,
                    InitValue::I32(0),
                )],
            ),
            (
                vec![
                    base.clone(),
                    value("other", ObjectType::I32, InitValue::I32(0), false),
                ],
                vec![
                    value("other", ObjectType::I32, InitValue::I32(0), false),
                    base,
                ],
            ),
        ];

        for (left, right) in incompatible {
            assert_incompatible(left, right);
        }
    }

    #[test]
    fn nested_reserved_state_class_is_rejected() {
        let states = StateType::SubState(
            "root".to_string(),
            "Root",
            vec![StateType::SubState(
                "reserved".to_string(),
                "StatesServer",
                Vec::new(),
            )],
        );

        let result = std::panic::catch_unwind(|| validate_states(&states));
        assert!(result.is_err(), "Expected reserved class to be rejected");
    }
}
