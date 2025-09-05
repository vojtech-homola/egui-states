use std::collections::{HashMap, VecDeque};
use std::string::ToString;
use std::{fs, io::Write};

pub struct EnumParse {
    name: String,
    variants: Vec<(String, i64)>,
}

pub fn read_enums(file_path: impl ToString) -> Vec<EnumParse> {
    let mut lines: VecDeque<String> = fs::read_to_string(file_path.to_string())
        .unwrap()
        .lines()
        .map(String::from)
        .collect();

    let mut result = Vec::new();
    while lines.len() > 0 {
        let line = lines.pop_front().unwrap();

        if line.contains("pub enum") || line.contains("pub(crate) enum") {
            let enum_name = line.split(" ").collect::<Vec<&str>>()[2];
            let mut enum_parse = EnumParse {
                name: enum_name.to_string(),
                variants: Vec::new(),
            };

            let mut counter = 0i64;
            loop {
                let line = lines.pop_front().unwrap();

                if line.contains("#") {
                    continue;
                } else if line.contains("}") {
                    break;
                } else {
                    let line = line.replace(",", "").trim().to_string();
                    if line.contains("=") {
                        let name = line.split("=").collect::<Vec<&str>>()[0].trim().to_string();
                        let value = line.split("=").collect::<Vec<&str>>()[1]
                            .trim()
                            .parse::<i64>()
                            .unwrap();
                        enum_parse.variants.push((name, value));
                        counter = value;
                    } else {
                        let name = line.trim().to_string();
                        enum_parse.variants.push((name, counter));
                    }
                    counter += 1;
                }
            }

            result.push(enum_parse);
        }
    }

    result
}

// custem types ----------------------------------------------------------------
pub struct StructParse {
    name: String,
    fields: Vec<(String, (String, String))>,
}

pub fn read_structs(file_path: impl ToString) -> Vec<StructParse> {
    let mut lines: VecDeque<String> = fs::read_to_string(file_path.to_string())
        .unwrap()
        .lines()
        .map(String::from)
        .collect();

    let mut result = Vec::new();
    while lines.len() > 0 {
        let line = lines.pop_front().unwrap();

        if line.contains("pub struct") || line.contains("pub(crate) struct") {
            let struct_name = line.split(" ").collect::<Vec<&str>>()[2];
            let mut struct_parse = StructParse {
                name: struct_name.to_string(),
                fields: Vec::new(),
            };

            loop {
                let line = lines.pop_front().unwrap();

                if line.contains("#") {
                    continue;
                } else if line.contains("}") {
                    break;
                } else {
                    let line = line.replace(",", "").trim().to_string();
                    let name = line.split(": ").collect::<Vec<&str>>()[0]
                        .trim()
                        .to_string();
                    let name = name
                        .split(" ")
                        .collect::<Vec<&str>>()
                        .last()
                        .unwrap()
                        .trim()
                        .to_string();
                    let item_type = line.split(": ").collect::<Vec<&str>>()[1]
                        .trim()
                        .to_string();
                    let item_type_parse = parse_types(&item_type, &None).unwrap();
                    struct_parse
                        .fields
                        .push((name, (item_type, item_type_parse)));
                }
            }

            result.push(struct_parse);
        }
    }

    result
}

// states -----------------------------------------------------------------------
#[derive(PartialEq)]
enum ValueType {
    Value,
    ValueStatic,
    ValueImage,
    Signal,
    ValueDict,
    ValueList,
    ValueGraphs,
}

impl ValueType {
    fn as_add_str(&self) -> &'static str {
        match self {
            ValueType::Value => "add_value",
            ValueType::ValueStatic => "add_static",
            ValueType::ValueImage => "add_image",
            ValueType::Signal => "add_signal",
            ValueType::ValueDict => "add_dict",
            ValueType::ValueList => "add_list",
            ValueType::ValueGraphs => "add_graphs",
        }
    }
}

struct Value {
    typ: ValueType,
    default: String,
    annotation: String,
}

impl Value {
    fn new(definition: String, declaration: String) -> Self {
        let typ = if definition.contains("ValueStatic") {
            ValueType::ValueStatic
        } else if definition.contains("<ValueImage>") {
            ValueType::ValueImage
        } else if definition.contains("<Signal<") {
            ValueType::Signal
        } else if definition.contains("<ValueDict<") {
            ValueType::ValueDict
        } else if definition.contains("<ValueList<") {
            ValueType::ValueList
        } else if definition.contains("<ValueGraphs<") {
            ValueType::ValueGraphs
        } else if definition.contains("<Value<") {
            ValueType::Value
        } else {
            panic!("Unknown value type: {}", definition);
        };

        let annot = if let ValueType::ValueImage = typ {
            "".to_string()
        } else {
            let annot = definition.split("<").collect::<Vec<&str>>()[2];
            annot.split(">").collect::<Vec<&str>>()[0].to_string()
        };

        let first = declaration.find("(").unwrap();
        let default = declaration[first + 1..].to_string();
        let last = default.rfind(")").unwrap();
        let default = default[..last].to_string();

        let default = if typ == ValueType::ValueList || typ == ValueType::ValueDict {
            "".to_string()
        } else {
            default
        };

        Self {
            typ,
            default,
            annotation: annot,
        }
    }
}

enum Item {
    Value(String, Value),
    State(String, State),
}

#[inline]
fn test_if_value(line: &str) -> bool {
    line.contains("Arc<Value<")
        || line.contains("Arc<ValueStatic<")
        || line.contains("Arc<ValueImage>")
        || line.contains("Arc<ValueGraphs<")
        || line.contains("Arc<Signal<")
        || line.contains("Arc<ValueDict<")
        || line.contains("Arc<ValueList<")
}

struct State {
    name: String,
    items: Vec<Item>,
}

impl State {
    fn new(name: String, lines: &Vec<String>) -> Result<Self, String> {
        let mut values = HashMap::new();
        let mut substates = HashMap::new();

        // process definition of the structs
        let mut started = false;
        let mut finished = false;
        for line in lines {
            if line.contains(format!("struct {}", name).as_str()) {
                started = true;
                continue;
            }

            if started {
                if line.contains("}") {
                    finished = true;
                    break;
                } else if line.contains("{") || line.trim().is_empty() || line.contains("//") {
                    continue;
                } else if test_if_value(line) {
                    let item_name = line.split(": ").collect::<Vec<&str>>()[0];
                    let item_name = item_name
                        .split(" ")
                        .collect::<Vec<&str>>()
                        .last()
                        .unwrap()
                        .to_string();
                    let item = line.split(": ").collect::<Vec<&str>>()[1];
                    let item = item[..item.len() - 1].to_string(); // remove comma
                    values.insert(item_name, item);
                } else {
                    let item_name = line.split(": ").collect::<Vec<&str>>()[0];
                    let item_name = item_name
                        .split(" ")
                        .collect::<Vec<&str>>()
                        .last()
                        .unwrap()
                        .to_string();
                    let item = line.split(": ").collect::<Vec<&str>>()[1];
                    let item = item[..item.len() - 1].to_string(); // remove comma

                    let state = State::new(item, lines);
                    if let Ok(state) = state {
                        substates.insert(item_name, state);
                    }
                }
            }
        }

        if !finished {
            return Err(format!("Failed to parse state: {}", name));
        }

        // process impl of the structs
        let mut items = Vec::new();
        let mut started = false;
        let mut finished = false;

        for line in lines {
            if line.contains(format!("impl {}", name).as_str()) {
                started = true;
                continue;
            }

            if started {
                if line == "}" {
                    finished = true;
                    break;
                }

                let mut key = "".to_string();
                for name in substates.keys() {
                    if line.contains(format!(" {}:", name).as_str()) {
                        key = name.clone();
                        break;
                    }
                }

                if !key.is_empty() {
                    let (name, state) = substates.remove_entry(&key).unwrap();
                    items.push(Item::State(name, state));
                    continue;
                }

                let mut key = "".to_string();
                for name in values.keys() {
                    if line.contains(format!(" {}:", name).as_str()) {
                        key = name.clone();
                        break;
                    }
                }

                if !key.is_empty() {
                    let (name, definition) = values.remove_entry(&key).unwrap();
                    let value = Value::new(definition, line.clone());
                    items.push(Item::Value(name, value));
                }
            }
        }

        if !finished || items.is_empty() {
            return Err(format!("Failed to parse state: {}", name));
        }

        Ok(Self { name, items })
    }

    fn write_python(
        &self,
        file: &mut fs::File,
        core: &Option<String>,
        written: &mut Vec<String>,
        root: bool,
    ) {
        for item in &self.items {
            if let Item::State(_, state) = item {
                state.write_python(file, core, written, false);
            }
        }

        if root {
            file.write_all(format!("\n\nclass {}(sc._MainStatesBase):\n", self.name).as_bytes())
                .unwrap();
            file.write_all(b"    def __init__(self, update: Callable[[float | None], None]):\n")
                .unwrap();
            file.write_all(b"        self._update = update\n").unwrap();
            file.write_all(b"        c = sc._Counter()\n\n").unwrap();
        } else if !written.contains(&self.name) {
            file.write_all(format!("\n\nclass {}(sc._StatesBase):\n", self.name).as_bytes())
                .unwrap();
            file.write_all(b"    def __init__(self, c: sc._Counter):\n")
                .unwrap();
            written.push(self.name.clone());
        } else {
            return;
        }

        let core = core.clone();
        for item in &self.items {
            match item {
                Item::Value(name, value) => {
                    let text = match value.typ {
                        ValueType::Value => {
                            let val_type = parse_types(&value.annotation, &core).unwrap();
                            format!("        self.{} = sc.Value[{}](c)\n", name, val_type)
                        }
                        ValueType::ValueStatic => {
                            let val_type = parse_types(&value.annotation, &core).unwrap();
                            format!("        self.{} = sc.ValueStatic[{}](c)\n", name, val_type)
                        }
                        ValueType::ValueImage => {
                            format!("        self.{} = sc.ValueImage(c)\n", name)
                        }
                        ValueType::Signal => {
                            let val_type = parse_types(&value.annotation, &core).unwrap();
                            if value.annotation == "Empty" {
                                format!("        self.{} = sc.SignalEmpty(c)\n", name)
                            } else {
                                format!("        self.{} = sc.Signal[{}](c)\n", name, val_type)
                            }
                        }
                        ValueType::ValueDict => {
                            let key_type = value.annotation.split(",").collect::<Vec<&str>>()[0];
                            let val_type =
                                value.annotation.split(",").collect::<Vec<&str>>()[1].trim();
                            let key_type = parse_types(key_type, &core).unwrap();
                            let val_type = parse_types(val_type, &core).unwrap();
                            format!(
                                "        self.{} = sc.ValueDict[{}, {}](c)\n",
                                name, key_type, val_type
                            )
                        }
                        ValueType::ValueList => {
                            let val_type = parse_types(&value.annotation, &core).unwrap();
                            format!("        self.{} = sc.ValueList[{}](c)\n", name, val_type)
                        }
                        ValueType::ValueGraphs => {
                            format!("        self.{} = sc.ValueGraphs(c)\n", name)
                        }
                    };

                    file.write_all(text.as_bytes()).unwrap();
                }
                Item::State(name, state) => {
                    let text = format!("        self.{} = {}(c)\n", name, state.name);
                    file.write_all(text.as_bytes()).unwrap();
                }
            }
        }

        if root {
            file.write_all(b"\n    def update(self, duration: float | None = None) -> None:\n")
                .unwrap();
            file.write_all(b"        \"\"\"Update the UI.\n\n").unwrap();
            file.write_all(b"        Args:\n").unwrap();
            file.write_all(b"            duration (float | None): The duration of the update.\n")
                .unwrap();
            file.write_all(b"        \"\"\"\n").unwrap();
            file.write_all(b"        self._update(duration)\n").unwrap();
        }

        // file.write_all(b"\n").unwrap();
    }
}

// states for server -----------------------------------------------------------
pub fn parse_states_for_server(
    states_file: impl ToString,
    output_file: impl ToString,
    root_state: &'static str,
    enums: &Option<Vec<EnumParse>>,
    structs: &Option<Vec<StructParse>>,
    replace: Vec<String>,
) -> Result<(), String> {
    let lines: Vec<String> = fs::read_to_string(states_file.to_string())
        .map_err(|e| format!("Failed to read file: {}", e))?
        .lines()
        .map(String::from)
        .collect();

    let state = State::new(root_state.to_string(), &lines)?;

    let mut file = fs::File::create(output_file.to_string())
        .map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"// Ganerated by build.rs, do not edit\n")
        .unwrap();

    if enums.is_some() || structs.is_some() {
        file.write_all(b"use serde::{Deserialize, Serialize};\n")
            .unwrap();
    }

    file.write_all(b"\nuse egui_states_pyserver::ServerValuesCreator;\n")
        .unwrap();

    if enums.is_some() {
        file.write_all(b"use egui_states_pyserver::pyenum;\n").unwrap();
    }

    if structs.is_some() {
        file.write_all(b"use egui_states_pyserver::pystruct;\n").unwrap();
    }

    // let mut has_empty = false;
    // for item in &state.items {
    //     if let Item::Value(_, value) = item {
    //         if value.annotation == "Empty" {
    //             has_empty = true;
    //             break;
    //         }
    //     }
    // }
    // if has_empty {
    //     file.write_all(b"use egui_pysync::Empty;\n\n").unwrap();
    // } else {
    file.write_all(b"\n").unwrap();
    // }

    file.write_all(b"pub(crate) fn create_states(c: &mut ServerValuesCreator) {\n")
        .unwrap();

    fn write_values(file: &mut fs::File, items: &Vec<Item>, replace: &Vec<String>) {
        for item in items {
            match item {
                Item::Value(_, value) => {
                    let add_str = value.typ.as_add_str();

                    let mut default = value.default.clone();
                    let mut annotation = value.annotation.clone();

                    for rep in replace {
                        let to_replcae = format!("{}::", rep);
                        default = default.replace(&to_replcae, "");
                    }
                    for rep in replace {
                        let to_replcae = format!("{}::", rep);
                        annotation = annotation.replace(&to_replcae, "");
                    }

                    let text = if annotation.is_empty() {
                        format!("    c.{}({});\n", add_str, default)
                    } else if add_str == "add_signal" {
                        format!("    c.{}::<{}>();\n", add_str, annotation)
                    } else {
                        format!("    c.{}::<{}>({});\n", add_str, annotation, default)
                    };
                    file.write_all(text.as_bytes()).unwrap();
                }
                Item::State(_, state) => {
                    write_values(file, &state.items, replace);
                }
            }
        }
    }

    write_values(&mut file, &state.items, &replace);

    file.write_all(b"}\n").unwrap();

    if enums.is_some() || structs.is_some() {
        // register types
        file.write_all(
        b"\npub(crate) fn register_types(m: &egui_states_pyserver::pyo3::Bound<egui_states_pyserver::pyo3::types::PyModule>) -> egui_states_pyserver::pyo3::PyResult<()> {\n",
    )
    .unwrap();
        file.write_all(b"    use egui_states_pyserver::pyo3::prelude::*;\n\n")
            .unwrap();
        if let Some(enums) = enums {
            for en in enums {
                let text = format!("    m.add_class::<{}>()?;\n", en.name);
                file.write_all(text.as_bytes()).unwrap();
            }
        }

        if let Some(structs) = structs {
            for st in structs {
                let text = format!("    m.add_class::<{}>()?;\n", st.name);
                file.write_all(text.as_bytes()).unwrap();
            }
        }
        file.write_all(b"\n    Ok(())\n").unwrap();
        file.write_all(b"}\n\n").unwrap();

        if let Some(enums) = enums {
            for en in enums {
                file.write_all(b"#[pyenum]\n").unwrap();
                file.write_all(b"#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]\n")
                    .unwrap();
                file.write_all(format!("enum {} ", en.name).as_bytes())
                    .unwrap();
                file.write_all(b"{\n").unwrap();

                for (name, value) in &en.variants {
                    let text = format!("    {} = {},\n", name, value);
                    file.write_all(text.as_bytes()).unwrap();
                }
                file.write_all(b"}\n\n").unwrap();
            }
        }

        if let Some(structs) = structs {
            for st in structs {
                file.write_all(b"#[pystruct]\n").unwrap();
                file.write_all(b"#[derive(Clone, Serialize, Deserialize)]\n")
                    .unwrap();
                file.write_all(format!("struct {} {{\n", st.name).as_bytes())
                    .unwrap();
                for (name, typ) in &st.fields {
                    let mut typ = typ.0.clone();
                    for rep in &replace {
                        let to_replcae = format!("{}::", rep);
                        typ = typ.replace(&to_replcae, "");
                    }
                    let text = format!("    pub {}: {},\n", name, typ);
                    file.write_all(text.as_bytes()).unwrap();
                }
                file.write_all(b"}\n\n").unwrap();
            }
        }
    }

    Ok(())
}

// states for client -----------------------------------------------------------
fn type_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("u8", "int");
    map.insert("u16", "int");
    map.insert("u32", "int");
    map.insert("u64", "int");
    map.insert("usize", "int");
    map.insert("i8", "int");
    map.insert("i16", "int");
    map.insert("i32", "int");
    map.insert("i64", "int");
    map.insert("isize", "int");
    map.insert("f32", "float");
    map.insert("f64", "float");
    map.insert("bool", "bool");
    map.insert("String", "str");
    map.insert("Empty", "Empty");
    map
}

fn parse_types(value: &str, core: &Option<String>) -> Result<String, String> {
    let map = type_map();

    if let Some(v) = map.get(value) {
        return Ok(v.to_string());
    }

    if value == "()" {
        return Ok("".to_string());
    }

    if value.starts_with("[") && value.ends_with("]") {
        let val = value[1..value.len() - 1].to_string();
        let typ_val = val.split(";").collect::<Vec<&str>>()[0].trim();
        let typ_val = parse_types(typ_val, core)?;
        return Ok(format!("list[{}]", typ_val));
    }

    if value.starts_with("(") && value.ends_with(")") {
        let val = value[1..value.len() - 1].to_string();
        let vals = val.split(",").collect::<Vec<&str>>();
        let vals_array = vals
            .iter()
            .map(|val| parse_types(val.trim(), core))
            .collect::<Result<Vec<String>, String>>()?;
        let text = vals_array.join(", ");
        return Ok(format!("tuple[{}]", text));
    }

    if value.contains("::") {
        let val = value
            .split("::")
            .collect::<Vec<&str>>()
            .last()
            .unwrap()
            .to_string();

        let val = match core {
            Some(core) => format!("{}.{}", core, val),
            None => val,
        };

        return Ok(val);
    }

    Err(format!("Unknown type: {}", value))
}

pub fn parse_states_for_client(
    state_file: impl ToString,
    output_file: impl ToString,
    root_state: &'static str,
    package_name: Option<String>,
    core: Option<String>,
) -> Result<(), String> {
    let lines: Vec<String> = fs::read_to_string(state_file.to_string())
        .map_err(|e| format!("Failed to read file: {}", e))?
        .lines()
        .map(String::from)
        .collect();

    let state = State::new(root_state.to_string(), &lines)?;

    let mut file = fs::File::create(output_file.to_string())
        .map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"# Ganerated by build.rs, do not edit\n")
        .unwrap();
    file.write_all(b"# ruff: noqa: D107 D101\n").unwrap();
    file.write_all(b"from collections.abc import Callable\n\n")
        .unwrap();
    file.write_all(b"from egui_pysync import structures as sc\n")
        .unwrap();

    if let Some(package_name) = package_name
        && let Some(core) = &core
    {
        let text = format!("\nfrom {} import {}", package_name, core);
        file.write_all(text.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
    }

    let mut written_classes = Vec::new();
    state.write_python(&mut file, &core, &mut written_classes, true);

    Ok(())
}

pub fn write_annotation(
    core: String,
    enums: Option<Vec<EnumParse>>,
    structs: Option<Vec<StructParse>>,
) {
    let mut file = fs::File::create(core)
        .map_err(|e| format!("Failed to create file: {}", e))
        .unwrap();

    file.write_all(b"# Ganerated by build.rs, do not edit\n")
        .unwrap();
    file.write_all(b"from egui_pysync.typing import SteteServerCoreBase")
        .unwrap();

    if enums.is_some() {
        file.write_all(b", PySyncEnum\n\n").unwrap();
    } else {
        file.write_all(b"\n\n").unwrap();
    }

    file.write_all(b"class StatesServerCore(SteteServerCoreBase):\n")
        .unwrap();
    file.write_all(b"    pass\n").unwrap();

    if let Some(ref enums) = enums {
        file.write_all(
            b"\n# enums ----------------------------------------------------------------",
        )
        .unwrap();
        for en in enums {
            file.write_all(format!("\nclass {}(PySyncEnum):\n", en.name).as_bytes())
                .unwrap();
            for item in &en.variants {
                let text = format!("    {} = {}\n", item.0, item.1);
                file.write_all(text.as_bytes()).unwrap();
            }
        }
    }

    if let Some(ref structs) = structs {
        file.write_all(
            b"\n# structs ----------------------------------------------------------------",
        )
        .unwrap();
        for st in structs {
            file.write_all(format!("\nclass {}:\n", st.name).as_bytes())
                .unwrap();
            let mut init = Vec::new();
            for item in &st.fields {
                let text = format!("    {}: {}\n", item.0, item.1.1);
                file.write_all(text.as_bytes()).unwrap();
                init.push(format!("{}: {}", item.0, item.1.1));
            }
            let t = init.join(", ");
            file.write_all(format!("\n    def __init__(self, {}):\n", t).as_bytes())
                .unwrap();
            file.write_all(b"        pass\n").unwrap();
        }
    }

    if structs.is_some() || enums.is_some() {
        file.write_all(b"\n__all__ = [\n").unwrap();

        if let Some(ref enums) = enums {
            for en in enums {
                file.write_all(format!("    \"{}\",\n", en.name).as_bytes())
                    .unwrap();
            }
        }

        if let Some(ref structs) = structs {
            for st in structs {
                file.write_all(format!("    \"{}\",\n", st.name).as_bytes())
                    .unwrap();
            }
        }

        file.write_all(b"]\n").unwrap();
    }

    file.flush().unwrap();
}
