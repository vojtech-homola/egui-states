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

fn state_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn state_definition(states: &[StateType], root: bool) -> String {
    let mut definition = if root {
        String::from("root|")
    } else {
        String::from("substate|")
    };

    for state in states {
        let item = match state {
            StateType::Value(name, typ, init, queue) => {
                format!("Value({:?},{typ:?},{init:?},{queue})", state_name(name))
            }
            StateType::ValueTake(name, typ) => {
                format!("ValueTake({:?},{typ:?})", state_name(name))
            }
            StateType::Static(name, typ, init) => {
                format!("Static({:?},{typ:?},{init:?})", state_name(name))
            }
            StateType::Image(name) => format!("Image({:?})", state_name(name)),
            StateType::ValueMap(name, key, value) => {
                format!("ValueMap({:?},{key:?},{value:?})", state_name(name))
            }
            StateType::ValueVec(name, typ) => {
                format!("ValueVec({:?},{typ:?})", state_name(name))
            }
            StateType::Signal(name, typ, queue) => {
                format!("Signal({:?},{typ:?},{queue})", state_name(name))
            }
            StateType::Data(name, typ) => format!("Data({:?},{typ:?})", state_name(name)),
            StateType::DataTake(name, typ) => {
                format!("DataTake({:?},{typ:?})", state_name(name))
            }
            StateType::DataMulti(name, typ) => {
                format!("DataMulti({:?},{typ:?})", state_name(name))
            }
            StateType::DataMultiTake(name, typ) => {
                format!("DataMultiTake({:?},{typ:?})", state_name(name))
            }
            StateType::SubState(name, class, _) => {
                format!("SubState({:?},{class:?})", state_name(name))
            }
        };
        definition.push_str(&item);
        definition.push('|');
    }

    definition
}

fn collect_state_definitions(
    state_class: &str,
    states: &[StateType],
    root: bool,
    definitions: &mut BTreeMap<String, String>,
) {
    if matches!(state_class, "StatesServer" | "enums" | "structs" | "s") {
        panic!("State {state_class} conflicts with a generated symbol");
    }

    let definition = state_definition(states, root);
    if let Some(previous) = definitions.get(state_class)
        && previous != &definition
    {
        panic!("State {state_class} defined multiple times with different fields");
    }
    definitions.insert(state_class.to_string(), definition);

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
