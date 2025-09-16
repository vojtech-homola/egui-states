use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::string::ToString;
use std::{fs, io::Write};

use crate::parser::{InitValue, TypeInfo, ValueType};
use crate::{ParseValuesCreator, State};

fn parse_states<S: State>() -> (BTreeMap<&'static str, Vec<ValueType>>, &'static str) {
    let mut creator = ParseValuesCreator::new();
    S::new(&mut creator);
    (creator.get_map(), S::N)
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
        InitValue::Array(elements) => {
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
    values: &[ValueType],
) -> (
    BTreeMap<&'static str, Vec<(&'static str, isize)>>,
    BTreeMap<&'static str, Vec<(&'static str, TypeInfo)>>,
) {
    let mut enums = BTreeMap::new();
    let mut structs = BTreeMap::new();

    for value in values {
        match value {
            ValueType::Value(_, _, info, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            ValueType::Static(_, _, info, _) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            ValueType::Dict(_, _, key_info, value_info) => {
                collect_enums(key_info, &mut enums);
                collect_enums(value_info, &mut enums);
                collect_structs(key_info, &mut structs);
                collect_structs(value_info, &mut structs);
            }
            ValueType::List(_, _, elem_info) => {
                collect_enums(elem_info, &mut enums);
                collect_structs(elem_info, &mut structs);
            }
            ValueType::Signal(_, _, info) => {
                collect_enums(info, &mut enums);
                collect_structs(info, &mut structs);
            }
            _ => { /* ignore other types */ }
        }
    }

    (enums, structs)
}

pub fn generate_rust_server<S: State>(path: impl ToString) -> Result<(), String> {
    let (map, _) = parse_states::<S>();

    let mut values_list = Vec::new();
    for (_, values) in map.iter() {
        for value in values {
            values_list.push(value.clone());
        }
    }

    let mut lines = Vec::new();
    for value in &values_list {
        match value {
            ValueType::Value(_, id, info, init) => {
                let line = format!(
                    "c.add_value::<{}>({}, {});",
                    type_info_to_type_string(info),
                    id,
                    init_to_string(init)
                );
                lines.push(line);
            }
            ValueType::Static(_, id, info, init) => {
                let line = format!(
                    "c.add_static::<{}>({}, {});",
                    type_info_to_type_string(info),
                    id,
                    init_to_string(init)
                );
                lines.push(line);
            }
            ValueType::Image(_, id) => {
                let line = format!("c.add_image({});", id);
                lines.push(line);
            }
            ValueType::Dict(_, id, key_info, value_info) => {
                let line = format!(
                    "c.add_dict::<{}, {}>({});",
                    type_info_to_type_string(key_info),
                    type_info_to_type_string(value_info),
                    id
                );
                lines.push(line);
            }
            ValueType::List(_, id, elem_info) => {
                let line = format!(
                    "c.add_list::<{}>({});",
                    type_info_to_type_string(elem_info),
                    id
                );
                lines.push(line);
            }
            ValueType::Graphs(_, id, elem_info) => {
                let line = format!(
                    "c.add_graphs::<{}>({});",
                    type_info_to_type_string(elem_info),
                    id
                );
                lines.push(line);
            }
            ValueType::Signal(_, id, info) => {
                let line = format!(
                    "c.add_signal::<{}>({});",
                    type_info_to_type_string(info),
                    id
                );
                lines.push(line);
            }
            ValueType::SubState(_, _) => { /* ignore */ }
        }
    }

    let (enums, structs) = get_all_enums_struct(&values_list);

    let mut file =
        fs::File::create(path.to_string()).map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"// Generated by build_scripts.rs, do not edit\n")
        .unwrap();

    if enums.len() > 0 || structs.len() > 0 {
        file.write_all(b"\nuse egui_states_pyserver::pyo3::prelude::*;\n")
            .unwrap();
    }

    file.write_all(b"use egui_states_pyserver::ServerValuesCreator;\n")
        .unwrap();

    if enums.len() > 0 {
        file.write_all(b"use egui_states_pyserver::pyenum;\n")
            .unwrap();
    }

    if structs.len() > 0 {
        file.write_all(b"use egui_states_pyserver::pystruct;\n")
            .unwrap();
    }

    file.write_all(b"\n").unwrap();

    file.write_all(b"pub(crate) fn create_states(c: &mut ServerValuesCreator) {\n")
        .unwrap();

    for line in lines {
        file.write_all(format!("    {}\n", line).as_bytes())
            .unwrap();
    }

    file.write_all(b"}\n").unwrap();

    if enums.len() > 0 || structs.len() > 0 {
        file.write_all(b"\npub(crate) fn register_types(m: &Bound<PyModule>) -> PyResult<()> {\n")
            .unwrap();

        for enum_name in enums.keys() {
            let text = format!("    m.add_class::<{}>()?;\n", enum_name);
            file.write_all(text.as_bytes()).unwrap();
        }

        for struct_name in structs.keys() {
            let text = format!("    m.add_class::<{}>()?;\n", struct_name);
            file.write_all(text.as_bytes()).unwrap();
        }

        file.write_all(b"    Ok(())\n").unwrap();
        file.write_all(b"}\n\n").unwrap();
    }

    for (enum_name, variants) in enums {
        file.write_all(b"#[pyenum]\n").unwrap();
        file.write_all(format!("enum {} ", enum_name).as_bytes())
            .unwrap();
        file.write_all(b"{\n").unwrap();

        for (name, value) in variants {
            let text = format!("    {} = {},\n", name, value);
            file.write_all(text.as_bytes()).unwrap();
        }
        file.write_all(b"}\n\n").unwrap();
    }

    for (struct_name, fields) in structs {
        file.write_all(b"#[pystruct]\n").unwrap();
        file.write_all(format!("struct {} {{\n", struct_name).as_bytes())
            .unwrap();
        for (name, typ) in &fields {
            let typ = type_info_to_type_string(typ);
            let text = format!("    pub {}: {},\n", name, typ);
            file.write_all(text.as_bytes()).unwrap();
        }
        file.write_all(b"}\n\n").unwrap();
    }

    Ok(())
}

fn type_info_to_python_type(info: &TypeInfo, import: Option<&impl ToString>) -> String {
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
                .map(|e| type_info_to_python_type(e, import))
                .collect();
            format!("tuple[{}]", elems.join(", "))
        }
        TypeInfo::Array(element, _) => {
            let elem_str = type_info_to_python_type(element, import);
            format!("list[{}]", elem_str)
        }
        TypeInfo::Option(element) => {
            let elem_str = type_info_to_python_type(element, import);
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
    path: impl ToString,
    import: Option<(impl ToString, impl ToString)>,
) -> Result<(), String> {
    let (map, root) = parse_states::<S>();

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

    let import_path = import.as_ref().map(|(_, p)| p);
    for (class_name, values) in map {
        if class_name == root {
            file.write_all(format!("\n\nclass {}(sc._MainStatesBase):\n", class_name).as_bytes())
                .unwrap();
            file.write_all(b"    def __init__(self, update: Callable[[float | None], None]):\n")
                .unwrap();
            file.write_all(b"        self._update = update\n\n")
                .unwrap();
        } else {
            file.write_all(format!("\n\nclass {}(sc._StatesBase):\n", class_name).as_bytes())
                .unwrap();
            file.write_all(b"    def __init__(self):\n").unwrap();
        }

        if values.len() == 0 {
            file.write_all(b"    pass\n").unwrap();
            continue;
        } else {
            for value in values {
                match value {
                    ValueType::Value(name, id, info, _) => {
                        let py_type = type_info_to_python_type(&info, import_path);
                        file.write_all(
                            format!(
                                "        self.{}: sc.Value[{}] = sc.Value({})\n",
                                name, py_type, id
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    ValueType::Static(name, id, info, _) => {
                        let py_type = type_info_to_python_type(&info, import_path);
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueStatic[{}] = sc.ValueStatic({})\n",
                                name, py_type, id
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    ValueType::Image(name, id) => {
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueImage = sc.ValueImage({})\n",
                                name, id
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    ValueType::Dict(name, id, key_info, value_info) => {
                        let py_key_type = type_info_to_python_type(&key_info, import_path);
                        let py_value_type = type_info_to_python_type(&value_info, import_path);
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueDict[{}, {}] = sc.ValueDict({})\n",
                                name, py_key_type, py_value_type, id
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    ValueType::List(name, id, info) => {
                        let py_type = type_info_to_python_type(&info, import_path);
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueList[{}] = sc.ValueList({})\n",
                                name, py_type, id
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    ValueType::Graphs(name, id, _) => {
                        file.write_all(
                            format!(
                                "        self.{}: sc.ValueGraphs = sc.ValueGraphs({})\n",
                                name, id
                            )
                            .as_bytes(),
                        )
                        .unwrap();
                    }
                    ValueType::Signal(name, id, info) => {
                        let py_type = type_info_to_python_type(&info, import_path);
                        let line = if py_type.is_empty() {
                            format!(
                                "        self.{}: sc.SignalEmpty = sc.SignalEmpty({})\n",
                                name, id
                            )
                        } else {
                            format!(
                                "        self.{}: sc.Signal[{}] = sc.Signal({})\n",
                                name, py_type, id
                            )
                        };

                        file.write_all(line.as_bytes()).unwrap();
                    }
                    ValueType::SubState(name, substate) => {
                        file.write_all(
                            format!("        self.{}: {} = {}()\n", name, substate, substate)
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
    let (map, _) = parse_states::<S>();
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
            let py_type = type_info_to_python_type(typ, None::<&String>);
            let text = format!("    {}: {}\n", name, py_type);
            file.write_all(text.as_bytes()).unwrap();
        }

        file.write_all(b"\n    def __init__(self,").unwrap();
        let elems: Vec<String> = fields
            .iter()
            .map(|(name, typ)| {
                let py_type = type_info_to_python_type(typ, None::<&String>);
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
