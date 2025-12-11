use std::collections::BTreeMap;
use std::string::ToString;

use egui_states_core::types::ObjectType;

use crate::State;
use crate::build_script::state_creator::StatesCreatorBuild;
use crate::build_script::values_info::StateType;

pub(crate) fn parse_states<S: State>() -> (BTreeMap<&'static str, Vec<StateType>>, &'static str) {
    let mut creator = StatesCreatorBuild::new();
    S::new(&mut creator, "root".to_string());
    let root_state = creator.root_state();
    (creator.get_states(), root_state)
}

fn collect_enums(type_info: &ObjectType, enums: &mut BTreeMap<String, Vec<(String, isize)>>) {
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
    BTreeMap<String, Vec<(String, isize)>>,
    BTreeMap<String, Vec<(String, ObjectType)>>,
) {
    let mut enums = BTreeMap::new();
    let mut structs = BTreeMap::new();

    for value in values {
        match value {
            StateType::Value(_, info, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            StateType::Static(_, info, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            StateType::Dict(_, key_info, value_info) => {
                collect_enums(key_info, &mut enums);
                collect_enums(value_info, &mut enums);
                collect_structs(key_info, &mut structs);
                collect_structs(value_info, &mut structs);
            }
            StateType::List(_, elem_info) => {
                collect_enums(elem_info, &mut enums);
                collect_structs(elem_info, &mut structs);
            }
            StateType::Signal(_, info) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            _ => { /* ignore other types */ }
        }
    }

    (enums, structs)
}
