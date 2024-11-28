use std::collections::{HashMap, VecDeque};
use std::string::ToString;
use std::{fs, io::Write};

// enums -----------------------------------------------------------------------
pub fn parse_enum(enum_path: impl ToString, output_path: impl ToString) -> Result<(), String> {
    let mut lines: VecDeque<String> = fs::read_to_string(enum_path.to_string())
        .map_err(|e| format!("Failed to read file: {}", e))?
        .lines()
        .map(String::from)
        .collect();

    let mut file = fs::File::create(output_path.to_string())
        .map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"# Ganerated by build.rs, do not edit\n")
        .unwrap();
    file.write_all(b"# ruff: noqa: D101\n").unwrap();
    file.write_all(b"from enum import Enum\n").unwrap();

    while lines.len() > 0 {
        let line = lines.pop_front().unwrap();

        if line.contains("pub enum") || line.contains("pub(crate) enum") {
            file.write_all(b"\n\n").unwrap();
            let enum_name = line.split(" ").collect::<Vec<&str>>()[2];
            file.write_all(format!("class {}(Enum):\n", enum_name).as_bytes())
                .unwrap();

            let mut counter = 0;
            loop {
                let line = lines.pop_front().unwrap();

                if line.contains("#") {
                    continue;
                } else if line.contains("}") {
                    break;
                } else {
                    let line = line.replace(",", "").trim().to_string();
                    if line.contains("=") {
                        file.write_all(format!("    {}\n", line).as_bytes())
                            .unwrap();
                    } else {
                        file.write_all(format!("    {} = {}\n", line, counter).as_bytes())
                            .unwrap();
                        counter += 1;
                    }
                }
            }
        }
    }

    file.flush().unwrap();
    Ok(())
}

// custem types ----------------------------------------------------------------
pub fn parse_custom_types(
    custom_types_path: impl ToString,
    output_path: impl ToString,
) -> Result<(), String> {
    let lines: Vec<String> = fs::read_to_string(custom_types_path.to_string())
        .map_err(|e| format!("Failed to read file: {}", e))?
        .lines()
        .map(String::from)
        .collect();

    let mut to_write: Vec<String> = Vec::new();

    fn parse_types(lines: &[String], to_write: &mut Vec<String>) {
        let line = lines[0].clone();

        if line.contains("//") {
            if line.contains("class") {
                let text = line.replace("//", "").trim().to_string();
                to_write.push(format!("\n{}\n", text));
                let mut i = 1;
                loop {
                    if lines[i].contains("//") {
                        let text = lines[i].replace("//", "").trim().to_string();
                        to_write.push(format!("    {}\n", text));
                        i += 1;
                    } else {
                        break;
                    }
                }
            } else {
                let text = line.replace("//", "").trim().to_string();
                to_write.push(format!("\n{}\n", text));
            }
        }
    }

    for (i, line) in lines.iter().enumerate() {
        if line.contains("#[derive") && !line.contains("//") {
            parse_types(&lines[i + 1..], &mut to_write);
        }
    }

    let mut file = fs::File::create(output_path.to_string())
        .map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"# Ganerated by build.rs, do not edit\n")
        .unwrap();
    file.write_all(b"# ruff: noqa: UP013 F403 F405 D101 E302 E305\n")
        .unwrap();
    file.write_all(b"from typing import *  # type: ignore\n")
        .unwrap();
    file.write_all(b"from collections.abc import *  # type: ignore\n\n")
        .unwrap();

    for line in to_write {
        file.write_all(line.as_bytes()).unwrap();
    }

    file.flush().unwrap();
    Ok(())
}

// states -----------------------------------------------------------------------
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

struct State {
    name: String,
    items: Vec<Item>,
}

impl State {
    fn write_python(
        &self,
        file: &mut fs::File,
        custom: &Option<(String, String)>,
        written: &mut Vec<String>,
        root: bool,
    ) {
        for item in &self.items {
            if let Item::State(_, state) = item {
                state.write_python(file, custom, written, false);
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

        for item in &self.items {
            match item {
                Item::Value(name, value) => {
                    let text = match value.typ {
                        ValueType::Value => {
                            let val_type = parse_types(&value.annotation, custom).unwrap();
                            if val_type.contains("enums.") {
                                format!(
                                    "        self.{} = sc.Value[{}](c, {})\n",
                                    name, val_type, val_type,
                                )
                            } else {
                                format!("        self.{} = sc.Value[{}](c)\n", name, val_type)
                            }
                        }
                        ValueType::ValueStatic => {
                            let val_type = parse_types(&value.annotation, custom).unwrap();
                            if val_type.contains("enums.") {
                                format!(
                                    "        self.{} = sc.ValueStatic[{}](c, {})\n",
                                    name, val_type, val_type,
                                )
                            } else {
                                format!("        self.{} = sc.ValueStatic[{}](c)\n", name, val_type)
                            }
                        }
                        ValueType::ValueImage => {
                            format!("        self.{} = sc.ValueImage(c)\n", name)
                        }
                        ValueType::Signal => {
                            let val_type = parse_types(&value.annotation, custom).unwrap();
                            if value.annotation == "()" {
                                format!("        self.{} = sc.SignalEmpty(c)\n", name)
                            } else if val_type.contains("enums.") {
                                format!(
                                    "        self.{} = sc.Signal[{}](c, {})\n",
                                    name, val_type, val_type
                                )
                            } else {
                                format!("        self.{} = sc.Signal[{}](c)\n", name, val_type)
                            }
                        }
                        ValueType::ValueDict => {
                            let key_type = value.annotation.split(",").collect::<Vec<&str>>()[0];
                            let val_type =
                                value.annotation.split(",").collect::<Vec<&str>>()[1].trim();
                            let key_type = parse_types(key_type, custom).unwrap();
                            let val_type = parse_types(val_type, custom).unwrap();
                            format!(
                                "        self.{} = sc.ValueDict[{}, {}](c)\n",
                                name, key_type, val_type
                            )
                        }
                        ValueType::ValueList => {
                            let val_type = parse_types(&value.annotation, custom).unwrap();
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
}

// states for server -----------------------------------------------------------
pub fn parse_states_for_server(
    states_file: impl ToString,
    output_file: impl ToString,
    root_state: &'static str,
    imports: Vec<&'static str>,
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
    for import in imports {
        file.write_all(format!("use {};\n", import).as_bytes())
            .unwrap();
    }
    file.write_all(b"\nuse egui_pyserver::ValuesCreator;\n\n")
        .unwrap();
    file.write_all(b"pub(crate) fn create_states(c: &mut ValuesCreator) {\n")
        .unwrap();

    fn write_values(file: &mut fs::File, items: &Vec<Item>) {
        for item in items {
            match item {
                Item::Value(_, value) => {
                    let add_str = value.typ.as_add_str();

                    let text = if value.annotation.is_empty() {
                        format!("    c.{}({});\n", add_str, value.default)
                    } else if value.annotation.contains("enums::")
                        && (add_str == "add_value" || add_str == "add_static")
                    {
                        format!(
                            "    c.{}_en::<{}>({});\n",
                            add_str, value.annotation, value.default
                        )
                    } else if value.annotation.contains("enums::") && add_str == "add_signal" {
                        format!("    c.{}::<u64>({});\n", add_str, value.default)
                    } else {
                        format!(
                            "    c.{}::<{}>({});\n",
                            add_str, value.annotation, value.default
                        )
                    };
                    file.write_all(text.as_bytes()).unwrap();
                }
                Item::State(_, state) => {
                    write_values(file, &state.items);
                }
            }
        }
    }

    write_values(&mut file, &state.items);

    file.write_all(b"}\n").unwrap();

    Ok(())
}

// states for client -----------------------------------------------------------
fn type_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("u8", "int");
    map.insert("u16", "int");
    map.insert("u32", "int");
    map.insert("u64", "int");
    map.insert("u128", "int");
    map.insert("usize", "int");
    map.insert("i8", "int");
    map.insert("i16", "int");
    map.insert("i32", "int");
    map.insert("i64", "int");
    map.insert("i128", "int");
    map.insert("isize", "int");
    map.insert("f32", "float");
    map.insert("f64", "float");
    map.insert("bool", "bool");
    map.insert("String", "str");
    map
}

fn parse_types(value: &str, custom: &Option<(String, String)>) -> Result<String, String> {
    let map = type_map();

    if let Some(v) = map.get(value) {
        return Ok(v.to_string());
    }

    if value == "()" {
        return Ok("".to_string());
    }

    if let Some((origin, python)) = custom {
        let origin = format!("{}::", origin);
        let python = format!("{}.", python);
        if value.contains(&origin) {
            return Ok(value.replace(&origin, &python));
        }
    }

    if value.contains("enums::") {
        return Ok(value.replace("::", "."));
    }

    if value.starts_with("[") && value.ends_with("]") {
        let val = value[1..value.len() - 1].to_string();
        if val.contains(";") {
            let typ_val = val.split(";").collect::<Vec<&str>>()[0].trim();
            let nums = val.split(";").collect::<Vec<&str>>()[1].trim();
            let typ_val = parse_types(typ_val, custom)?;
            let typ_vals = vec![typ_val; nums.parse::<usize>().unwrap()];
            let text = typ_vals.join(", ");
            return Ok(format!("tuple[{}]", text));
        } else {
            let vals = val.split(",").collect::<Vec<&str>>();
            let vals_array = vals
                .iter()
                .map(|val| parse_types(val.trim(), custom))
                .collect::<Result<Vec<String>, String>>()?;
            let text = vals_array.join(", ");
            return Ok(format!("tuple[{}]", text));
        }
    }

    if value.starts_with("(") && value.ends_with(")") {
        let val = value[1..value.len() - 1].to_string();
        let vals = val.split(",").collect::<Vec<&str>>();
        let vals_array = vals
            .iter()
            .map(|val| parse_types(val.trim(), custom))
            .collect::<Result<Vec<String>, String>>()?;
        let text = vals_array.join(", ");
        return Ok(format!("tuple[{}]", text));
    }

    Err(format!("Unknown type: {}", value))
}

pub fn parse_states_for_client(
    state_file: impl ToString,
    output_file: impl ToString,
    root_state: &'static str,
    imports: Vec<&'static str>,
    custom: Option<(String, String)>,
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
    file.write_all(b"from egui_pysync import structures as sc\n\n")
        .unwrap();
    for import in imports {
        file.write_all(import.as_bytes()).unwrap();
    }
    file.write_all(b"\n").unwrap();

    let mut written_classes = Vec::new();
    state.write_python(&mut file, &custom, &mut written_classes, true);

    Ok(())
}
