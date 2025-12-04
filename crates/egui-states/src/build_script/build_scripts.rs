use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::string::ToString;
use std::{fs, io::Write};

use crate::State;
use crate::build_script::values_info::{InitValue, TypeInfo, StateType};
use crate::build_script::state_creator::StatesCreatorBuild;

fn parse_states<S: State>() -> BTreeMap<&'static str, Vec<StateType>> {
    let mut creator = StatesCreatorBuild::new();
    S::new(&mut creator, "root".to_string());
    creator.get_states()
}

fn type_info_to_type_string(info: &TypeInfo) -> String {
    match info {
        TypeInfo::Basic(name) => name.to_string(),
        TypeInfo::Tuple(elements) => {
            let elems: Vec<String> = elements.iter().map(type_info_to_type_string).collect();
            format!("({})", elems.join(", "))
        }
        TypeInfo::Array(element, size) => {
            let elem_str = type_info_to_type_string(element);
            format!("[{}; {}]", elem_str, size)
        }
        TypeInfo::Option(element) => {
            let elem_str = type_info_to_type_string(element);
            format!("Option<{}>", elem_str)
        }
        TypeInfo::Struct(name, _) => name.to_string(),
        TypeInfo::Enum(name, _) => name.to_string(),
    }
}

fn init_to_string(init: &InitValue) -> String {
    match init {
        InitValue::Value(val) => val.clone(),
        InitValue::Option(opt) => match opt {
            Some(inner) => format!("Some({})", init_to_string(inner)),
            None => "None".to_string(),
        },
        InitValue::Struct(name, fields) => {
            let field_strs: Vec<String> = fields
                .iter()
                .map(|(name, value)| format!("{}: {}", name, init_to_string(value)))
                .collect();
            format!("{} {{ {} }}", name, field_strs.join(", "))
        }
        InitValue::Tuple(elements) => {
            let elem_strs: Vec<String> = elements.iter().map(init_to_string).collect();
            format!("({})", elem_strs.join(", "))
        }
        InitValue::List(elements) => {
            let elem_strs: Vec<String> = elements.iter().map(init_to_string).collect();
            format!("[{}]", elem_strs.join(", "))
        }
    }
}

fn collect_enums(
    type_info: &TypeInfo,
    enums: &mut BTreeMap<&'static str, Vec<(&'static str, isize)>>,
) {
    match type_info {
        TypeInfo::Enum(name, variants) => {
            if enums.contains_key(name) {
                if enums[name] != *variants {
                    panic!(
                        "Enum {} defined multiple times with different variants",
                        name
                    );
                }
            }

            enums.insert(name, variants.clone());
        }
        TypeInfo::Struct(_, fields) => {
            for (_, field_type) in fields {
                collect_enums(field_type, enums);
            }
        }
        TypeInfo::Tuple(elements) => {
            for elem in elements {
                collect_enums(elem, enums);
            }
        }
        TypeInfo::Array(element, _) => {
            collect_enums(element, enums);
        }
        TypeInfo::Option(element) => {
            collect_enums(element, enums);
        }
        TypeInfo::Basic(_) => { /* ignore basic types */ }
    }
}

fn collect_structs(
    type_info: &TypeInfo,
    structs: &mut BTreeMap<&'static str, Vec<(&'static str, TypeInfo)>>,
) {
    match type_info {
        TypeInfo::Struct(name, fields) => {
            if structs.contains_key(name) {
                if structs[name] != *fields {
                    panic!(
                        "Struct {} defined multiple times with different fields",
                        name
                    );
                }
            }

            structs.insert(name, fields.clone());
            for (_, field_type) in fields {
                collect_structs(field_type, structs);
            }
        }
        TypeInfo::Enum(_, variants) => {
            for (_, _) in variants {
                // Enums don't have nested types in this design
            }
        }
        TypeInfo::Tuple(elements) => {
            for elem in elements {
                collect_structs(elem, structs);
            }
        }
        TypeInfo::Array(element, _) => {
            collect_structs(element, structs);
        }
        TypeInfo::Option(element) => {
            collect_structs(element, structs);
        }
        TypeInfo::Basic(_) => { /* ignore basic types */ }
    }
}

fn get_all_enums_struct(
    values: &[StateType],
) -> (
    BTreeMap<&'static str, Vec<(&'static str, isize)>>,
    BTreeMap<&'static str, Vec<(&'static str, TypeInfo)>>,
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

fn type_info_to_python_type(
    info: &TypeInfo,
    import: Option<&impl ToString>,
    list_comment: bool,
) -> String {
    match info {
        TypeInfo::Basic(name) => match *name {
            "String" => "str".to_string(),
            "bool" => "bool".to_string(),
            "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64" => "int".to_string(),
            "f32" | "f64" => "float".to_string(),
            "()" => "".to_string(),
            _ => panic!("Unsupported basic type: {}", name),
        },
        TypeInfo::Tuple(elements) => {
            let elems: Vec<String> = elements
                .iter()
                .map(|e| type_info_to_python_type(e, import, list_comment))
                .collect();
            format!("tuple[{}]", elems.join(", "))
        }
        TypeInfo::Array(element, size) => {
            let elem_str = type_info_to_python_type(element, import, list_comment);
            if list_comment {
                format!("list[{}]  # size: {}", elem_str, size)
            } else {
                format!("list[{}]", elem_str)
            }
        }
        TypeInfo::Option(element) => {
            let elem_str = type_info_to_python_type(element, import, list_comment);
            format!("{} | None", elem_str)
        }
        TypeInfo::Struct(name, _) => match import {
            None => name.to_string(),
            Some(import) => {
                format!("{}.{}", import.to_string(), name)
            }
        },
        TypeInfo::Enum(name, _) => match import {
            None => name.to_string(),
            Some(import) => {
                format!("{}.{}", import.to_string(), name)
            }
        },
    }
}

pub fn generate_python_wrapper<S: State>(
    root: &'static str,
    path: impl ToString,
    import: Option<(impl ToString, impl ToString)>,
) -> Result<(), String> {
    let map = parse_states::<S>();
    // let root = map.get(root).unwrap();

    let mut values_list = Vec::new();
    for (_, values) in map.iter() {
        for value in values {
            values_list.push(value.clone());
        }
    }

    let mut file =
        fs::File::create(path.to_string()).map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"# Generated by build_scripts.rs, do not edit\n")
        .unwrap();
    file.write_all(b"# ruff: noqa: D107 D101\n").unwrap();
    file.write_all(b"from collections.abc import Callable\n\n")
        .unwrap();
    file.write_all(b"from egui_states import structures as sc\n")
        .unwrap();
    if let Some(import) = &import {
        file.write_all(
            format!(
                "from {} import {}\n",
                import.0.to_string(),
                import.1.to_string()
            )
            .as_bytes(),
        )
        .unwrap();
    }

    let mut used_names = Vec::new();
    let import_path = import.as_ref().map(|(_, p)| p);
    for (class_name, values) in map {
        if used_names.contains(&class_name) {
            continue;
        }
        used_names.push(class_name);

        if class_name == root {
            file.write_all(format!("\n\nclass {}(sc._MainStatesBase):\n", class_name).as_bytes())
                .unwrap();
            file.write_all(b"    def __init__(self, update: Callable[[float | None], None]):\n")
                .unwrap();
            file.write_all(b"        self._update = update\n").unwrap();
            file.write_all(b"        c = sc._Counter()\n\n").unwrap();
        } else {
            file.write_all(format!("\n\nclass {}(sc._StatesBase):\n", class_name).as_bytes())
                .unwrap();
            file.write_all(b"    def __init__(self, c: sc._Counter):\n")
                .unwrap();
        }

        if values.len() == 0 {
            file.write_all(b"    pass\n").unwrap();
            continue;
        } else {
            for value in values {
                match value {
                    StateType::Value(name, info, _) => {
                        let py_type = type_info_to_python_type(&info, import_path, false);
                        file.write_all(
                            format!(
                                "        self.{}: sc.Value[{}] = sc.Value(c)\n",
                                name, py_type
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    StateType::Static(name, info, _) => {
                        let py_type = type_info_to_python_type(&info, import_path, false);
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueStatic[{}] = sc.ValueStatic(c)\n",
                                name, py_type
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    StateType::Image(name) => {
                        file.write_all(
                            format!("        self.{}: sc.ValueImage = sc.ValueImage(c)\n", name)
                                .as_bytes(),
                        )
                        .unwrap();
                    }
                    StateType::Dict(name, key_info, value_info) => {
                        let py_key_type = type_info_to_python_type(&key_info, import_path, false);
                        let py_value_type =
                            type_info_to_python_type(&value_info, import_path, false);
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueDict[{}, {}] = sc.ValueDict(c)\n",
                                name, py_key_type, py_value_type
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    StateType::List(name, info) => {
                        let py_type = type_info_to_python_type(&info, import_path, false);
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueList[{}] = sc.ValueList(c)\n",
                                name, py_type
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    StateType::Graphs(name, _) => {
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueGraphs = sc.ValueGraphs(c)\n",
                                name
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    StateType::Signal(name, info) => {
                        let py_type = type_info_to_python_type(&info, import_path, false);
                        let line = if py_type.is_empty() {
                            format!(
                                "        self.{}: sc.SignalEmpty = sc.SignalEmpty(c)\n",
                                name
                            )
                        } else {
                            format!(
                                "        self.{}: sc.Signal[{}] = sc.Signal(c)\n",
                                name, py_type
                            )
                        };

                        file.write_all(line.as_bytes()).unwrap();
                    }
                    StateType::SubState(name, substate) => {
                        file.write_all(
                            format!("        self.{}: {} = {}(c)\n", name, substate, substate)
                                .as_bytes(),
                        )
                        .unwrap();
                    }
                }
            }
        }

        if class_name == root {
            file.write_all(b"\n    def update(self, duration: float | None = None) -> None:\n")
                .unwrap();
            file.write_all(b"        \"\"\"Update the UI.\n\n").unwrap();
            file.write_all(b"        Args:\n").unwrap();
            file.write_all(b"            duration (float | None): The duration of the update.\n")
                .unwrap();
            file.write_all(b"        \"\"\"\n").unwrap();
            file.write_all(b"        self._update(duration)\n").unwrap();
        }
    }

    Ok(())
}

fn order_structs(items: &Vec<(&'static str, TypeInfo)>, order: &mut VecDeque<&'static str>) {
    for (_, item_type) in items {
        if let TypeInfo::Struct(name, fields) = item_type {
            if !order.contains(name) {
                order.push_front(name);
                order_structs(fields, order);
            }
        }
    }
}

pub fn generate_pytypes<S: State>(path: impl ToString) -> Result<(), String> {
    let map = parse_states::<S>();
    let mut values_list = Vec::new();
    for (_, values) in map.iter() {
        for value in values {
            values_list.push(value.clone());
        }
    }
    let (enums, structs) = get_all_enums_struct(&values_list);
    let mut order_list = VecDeque::new();
    for (struct_name, items) in &structs {
        if !order_list.contains(struct_name) {
            order_list.push_front(struct_name);
            order_structs(items, &mut order_list);
        }
    }

    let mut file =
        fs::File::create(path.to_string()).map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"# Generated by build.rs, do not edit\n")
        .unwrap();

    file.write_all(b"from egui_states.typing import SteteServerCoreBase")
        .unwrap();

    if enums.len() > 0 {
        file.write_all(b", PySyncEnum\n\n").unwrap();
    } else {
        file.write_all(b"\n\n").unwrap();
    }

    file.write_all(b"class StatesServerCore(SteteServerCoreBase):\n")
        .unwrap();
    file.write_all(b"    pass\n").unwrap();

    for (enum_name, variants) in &enums {
        file.write_all(format!("\nclass {}(PySyncEnum):\n", enum_name).as_bytes())
            .unwrap();
        for (name, value) in variants {
            let text = format!("    {} = {}\n", name, value);
            file.write_all(text.as_bytes()).unwrap();
        }
    }

    if structs.len() > 0 {
        file.write_all(b"\n# Structs -------------------------------------------------")
            .unwrap();
    }

    for struct_name in order_list {
        let fields = &structs[struct_name];
        file.write_all(format!("\nclass {}:\n", struct_name).as_bytes())
            .unwrap();
        if fields.len() == 0 {
            file.write_all(b"    pass\n").unwrap();
            continue;
        }
        for (name, typ) in fields {
            let py_type = type_info_to_python_type(typ, None::<&String>, true);
            let text = format!("    {}: {}\n", name, py_type);
            file.write_all(text.as_bytes()).unwrap();
        }

        file.write_all(b"\n    def __init__(self,").unwrap();
        let elems: Vec<String> = fields
            .iter()
            .map(|(name, typ)| {
                let py_type = type_info_to_python_type(typ, None::<&String>, false);
                format!("{}: {}", name, py_type)
            })
            .collect();
        file.write_all(format!("{}) -> None:\n", elems.join(", ")).as_bytes())
            .unwrap();
        file.write_all(b"        pass\n").unwrap();
    }

    file.write_all(b"\n__all__ = [\n").unwrap();
    for enum_name in enums.keys() {
        file.write_all(format!("    \"{}\",\n", enum_name).as_bytes())
            .unwrap();
    }
    for struct_name in structs.keys() {
        file.write_all(format!("    \"{}\",\n", struct_name).as_bytes())
            .unwrap();
    }
    file.write_all(b"]\n").unwrap();

    Ok(())
}
