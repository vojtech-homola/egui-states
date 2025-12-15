use std::collections::HashMap;
use std::collections::VecDeque;
use std::string::ToString;
use std::{fs, io::Write};

use egui_states_core::graphs::GraphType;
use egui_states_core::types::ObjectType;

use crate::State;
use crate::build_script::scripts;
use crate::build_script::values_info::{InitValue, StateType};

fn type_to_pytype(type_info: &ObjectType) -> String {
    match type_info {
        ObjectType::U8 => "s.u8".to_string(),
        ObjectType::U16 => "s.u16".to_string(),
        ObjectType::U32 => "s.u32".to_string(),
        ObjectType::U64 => "s.u64".to_string(),
        ObjectType::I8 => "s.i8".to_string(),
        ObjectType::I16 => "s.i16".to_string(),
        ObjectType::I32 => "s.i32".to_string(),
        ObjectType::I64 => "s.i64".to_string(),
        ObjectType::F32 => "s.f32".to_string(),
        ObjectType::F64 => "s.f64".to_string(),
        ObjectType::Bool => "s.bo".to_string(),
        ObjectType::String => "s.st".to_string(),
        ObjectType::Empty => "s.emp".to_string(),
        ObjectType::Enum(name, _) => format!("s.enu({})", name),
        ObjectType::Struct(name, elments) => {
            let fields: Vec<String> = elments.iter().map(|(_, obj)| type_to_pytype(obj)).collect();
            format!("s.cl([{}], {})", fields.join(", "), name)
        }
        ObjectType::Tuple(vec) => {
            let elems: Vec<String> = vec.iter().map(|t| type_to_pytype(t)).collect();
            format!("s.tu([{}])", elems.join(", "))
        }
        ObjectType::List(size, element) => {
            format!("s.li({}, {})", type_to_pytype(element), size)
        }
        ObjectType::Vec(element) => {
            format!("s.vec({})", type_to_pytype(element))
        }
        ObjectType::Map(key_type, value_type) => {
            format!(
                "s.map({}, {})",
                type_to_pytype(key_type),
                type_to_pytype(value_type)
            )
        }
        ObjectType::Option(element) => {
            format!("s.opt({})", type_to_pytype(element))
        }
    }
}

fn process_type_info(values: &Vec<StateType>) -> (HashMap<String, usize>, Vec<ObjectType>) {
    let mut type_map: HashMap<String, usize> = HashMap::new();
    let mut type_list: Vec<ObjectType> = Vec::new();

    for state in values {
        match state {
            StateType::Value(name, obj_type, _)
            | StateType::Static(name, obj_type, _)
            | StateType::Signal(name, obj_type) => {
                if type_list.contains(obj_type) {
                    type_map.insert(
                        name.clone(),
                        type_list.iter().position(|t| t == obj_type).unwrap(),
                    );
                } else {
                    type_list.push(obj_type.clone());
                    type_map.insert(name.clone(), type_list.len() - 1);
                }
            }
            StateType::Map(name, key, value) => {
                let dict_type = ObjectType::Map(Box::new(key.clone()), Box::new(value.clone()));
                if type_list.contains(&dict_type) {
                    type_map.insert(
                        name.clone(),
                        type_list.iter().position(|t| t == &dict_type).unwrap(),
                    );
                } else {
                    type_list.push(dict_type);
                    type_map.insert(name.clone(), type_list.len() - 1);
                }
            }
            StateType::List(name, value_type) => {
                let list_type = ObjectType::Vec(Box::new(value_type.clone()));
                if type_list.contains(&list_type) {
                    type_map.insert(
                        name.clone(),
                        type_list.iter().position(|t| t == &list_type).unwrap(),
                    );
                } else {
                    type_list.push(list_type);
                    type_map.insert(name.clone(), type_list.len() - 1);
                }
            }
            _ => {}
        }
    }

    (type_map, type_list)
}

fn type_info_to_python_type(info: &ObjectType, list_comment: bool) -> String {
    match info {
        ObjectType::U8
        | ObjectType::U16
        | ObjectType::U32
        | ObjectType::U64
        | ObjectType::I8
        | ObjectType::I16
        | ObjectType::I32
        | ObjectType::I64 => "int".to_string(),
        ObjectType::F32 | ObjectType::F64 => "float".to_string(),
        ObjectType::Bool => "bool".to_string(),
        ObjectType::String => "str".to_string(),
        ObjectType::Empty => "".to_string(),
        ObjectType::Enum(name, _) => name.clone(),
        ObjectType::Struct(name, _) => name.clone(),
        ObjectType::Tuple(elements) => {
            let elems: Vec<String> = elements
                .iter()
                .map(|e| type_info_to_python_type(e, list_comment))
                .collect();
            format!("tuple[{}]", elems.join(", "))
        }
        ObjectType::List(size, element) => {
            let elem_str = type_info_to_python_type(element, list_comment);
            if list_comment {
                format!("list[{}]  # fixed size {}", elem_str, *size)
            } else {
                format!("list[{}]", elem_str)
            }
        }
        ObjectType::Vec(t) => {
            let elem_str = type_info_to_python_type(t, list_comment);
            format!("list[{}]", elem_str)
        }
        ObjectType::Map(key, value) => {
            let key_str = type_info_to_python_type(key, list_comment);
            let value_str = type_info_to_python_type(value, list_comment);
            format!("dict[{}, {}]", key_str, value_str)
        }
        ObjectType::Option(element) => {
            let elem_str = type_info_to_python_type(element, list_comment);
            format!("{} | None", elem_str)
        }
    }
}

fn init_to_python_value(init: &InitValue) -> String {
    match init {
        InitValue::U8(v) => format!("{}", v),
        InitValue::U16(v) => format!("{}", v),
        InitValue::U32(v) => format!("{}", v),
        InitValue::U64(v) => format!("{}", v),
        InitValue::I8(v) => format!("{}", v),
        InitValue::I16(v) => format!("{}", v),
        InitValue::I32(v) => format!("{}", v),
        InitValue::I64(v) => format!("{}", v),
        InitValue::F64(v) => format!("{}", v),
        InitValue::F32(v) => format!("{}", v),
        InitValue::String(v) => format!("\"{}\"", v),
        InitValue::Bool(v) => format!("{}", v),
        InitValue::Enum(v) => {
            let value = v.replace("::", ".");
            format!("{}", value)
        }
        InitValue::Option(opt) => match opt {
            Some(boxed) => init_to_python_value(boxed),
            None => "None".to_string(),
        },
        InitValue::Tuple(elems) => {
            let elem_strs: Vec<String> = elems.iter().map(|e| init_to_python_value(e)).collect();
            format!("({})", elem_strs.join(", "))
        }
        InitValue::List(elems) | InitValue::Vec(elems) => {
            let elem_strs: Vec<String> = elems.iter().map(|e| init_to_python_value(e)).collect();
            format!("[{}]", elem_strs.join(", "))
        }
        InitValue::Map(pairs) => {
            let pair_strs: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", init_to_python_value(k), init_to_python_value(v)))
                .collect();
            format!("{{{}}}", pair_strs.join(", "))
        }
        InitValue::Struct(name, items) => {
            let field_strs: Vec<String> = items
                .iter()
                .map(|(_, value)| init_to_python_value(value))
                .collect();
            format!("{}({})", name, field_strs.join(", "))
        }
    }
}

fn state_to_line(state: &StateType, types_map: &HashMap<String, usize>) -> String {
    match state {
        StateType::Value(name, state_type, init) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let init_value = init_to_python_value(init);
            let index = types_map.get(name).unwrap();
            format!(
                "        self.{} = s.Value[{}]({}, {})\n",
                last_name, py_type, *index, init_value
            )
        }
        StateType::Static(name, state_type, init) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let init_value = init_to_python_value(init);
            let index = types_map.get(name).unwrap();
            format!(
                "        self.{} = s.ValueStatic[{}]({}, {})\n",
                last_name, py_type, *index, init_value
            )
        }
        StateType::Signal(name, state_type) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let index = types_map.get(name).unwrap();
            if let ObjectType::Empty = state_type {
                format!("        self.{} = s.SignalEmpty()\n", last_name)
            } else {
                format!(
                    "        self.{} = s.Signal[{}]({})\n",
                    last_name, py_type, *index
                )
            }
        }
        StateType::List(name, state_type) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let index = types_map.get(name).unwrap();
            format!(
                "        self.{} = s.ValueList[{}]({})\n",
                last_name, py_type, *index
            )
        }
        StateType::Map(name, key_type, value_type) => {
            let last_name = name.split('.').last().unwrap();
            let py_key_type = type_info_to_python_type(key_type, false);
            let py_value_type = type_info_to_python_type(value_type, false);
            let index = types_map.get(name).unwrap();
            format!(
                "        self.{} = s.ValueMap[{}, {}]({})\n",
                last_name, py_key_type, py_value_type, *index
            )
        }
        StateType::Graphs(name, graph_type) => {
            let last_name = name.split('.').last().unwrap();
            format!(
                "        self.{} = s.ValueGraphs({})\n",
                last_name,
                match graph_type {
                    GraphType::F32 => "np.float32",
                    GraphType::F64 => "np.float64",
                }
            )
        }
        StateType::Image(name) => {
            let last_name = name.split('.').last().unwrap();
            format!("        self.{} = s.ValueImage()\n", last_name)
        }
        StateType::SubState(name, state_class, _) => {
            let last_name = name.split('.').last().unwrap();
            format!(
                "        self.{} = {}(parent + \".{}\")\n",
                last_name, state_class, last_name
            )
        }
    }
}

fn write_states(
    file: &mut fs::File,
    state_class: &str,
    states: &Vec<StateType>,
    types_map: &HashMap<String, usize>,
) {
    let mut lines = Vec::new();

    for state in states {
        lines.push(state_to_line(state, types_map));
        if let StateType::SubState(_, state_class, sub_states) = state {
            write_states(file, state_class, sub_states, types_map);
        }
    }

    file.write_all(format!("\n\nclass {}(ISubStates):\n", state_class).as_bytes())
        .unwrap();
    file.write_all(b"    def __init__(self, parent: str):\n")
        .unwrap();

    for line in lines {
        file.write_all(line.as_bytes()).unwrap();
    }
}

fn order_structs(items: &Vec<(String, ObjectType)>, order: &mut VecDeque<String>) {
    for (_, item_type) in items {
        if let ObjectType::Struct(name, fields) = item_type {
            if !order.contains(name) {
                order.push_front(name.clone());
                order_structs(fields, order);
            }
        }
    }
}

pub fn generate<S: State>(path: impl ToString) -> Result<(), String> {
    let states = scripts::parse_states::<S>();

    let mut values_list = Vec::new();
    scripts::states_into_values_list(&states, &mut values_list);
    let (enums, structs) = scripts::get_all_enums_struct(&values_list);
    let mut order_list = VecDeque::new();
    for (struct_name, items) in &structs {
        if !order_list.contains(struct_name) {
            order_list.push_front(struct_name.clone());
            order_structs(items, &mut order_list);
        }
    }
    let (types_map, types_list) = process_type_info(&values_list);

    let mut file =
        fs::File::create(path.to_string()).map_err(|e| format!("Failed to create file: {}", e))?;

    file.write_all(b"# Generated by build.rs, do not edit\n")
        .unwrap();
    file.write_all(b"# ruff: noqa: D101 D107\n").unwrap();
    file.write_all(b"from collections.abc import Callable\n")
        .unwrap();
    if structs.len() > 0 {
        file.write_all(b"from dataclasses import dataclass\n")
            .unwrap();
    }

    file.write_all(b"\nimport numpy as np\n\n").unwrap();

    file.write_all(b"import egui_states.structures as s\n")
        .unwrap();
    file.write_all(b"from egui_states.server import StatesBase, StateServerBase\n")
        .unwrap();
    file.write_all(b"from egui_states.structures import ISubStates\n")
        .unwrap();

    // Write enums
    for (enum_name, variants) in &enums {
        file.write_all(format!("\n\nclass {}(s.FastEnum):\n", enum_name).as_bytes())
            .unwrap();
        for (name, value) in variants {
            let text = format!("    {} = {}\n", name, value);
            file.write_all(text.as_bytes()).unwrap();
        }
    }

    // Write custom structs
    for struct_name in &order_list {
        let fields = &structs[struct_name];
        file.write_all(
            format!("\n\n@dataclass\nclass {}(s.CustomStruct):\n", struct_name).as_bytes(),
        )
        .unwrap();

        if fields.len() == 0 {
            file.write_all(b"    pass\n").unwrap();
            continue;
        }

        for (name, typ) in fields {
            let py_type = type_info_to_python_type(typ, true);
            let text = format!("    {}: {}\n", name, py_type);
            file.write_all(text.as_bytes()).unwrap();
        }
    }

    // write states
    if let StateType::SubState(_, root_name, substates) = &states {
        // write substates
        for state in substates {
            if let StateType::SubState(_, state_class, sub_states) = state {
                write_states(&mut file, state_class, sub_states, &types_map);
            }
        }

        // write root state
        // Write the _get_obj_types function
        file.write_all(format!("\n\nclass {}(StatesBase):\n", root_name).as_bytes())
            .unwrap();
        file.write_all(b"    @staticmethod\n").unwrap();
        file.write_all(b"    def _get_obj_types() -> list[s.PyObjectType]:\n")
            .unwrap();
        file.write_all(b"        return [\n").unwrap();
        for obj_type in &types_list {
            let py_type_str = type_to_pytype(obj_type);
            file.write_all(format!("            {},\n", py_type_str).as_bytes())
                .unwrap();
        }
        file.write_all(b"        ]\n\n").unwrap();

        // Write the state values
        file.write_all(b"    def __init__(self, server: StateServerBase):\n")
            .unwrap();
        file.write_all(b"        super().__init__(server)\n")
            .unwrap();
        file.write_all(b"        parent = \"root\"\n").unwrap();

        for state in substates {
            let line = state_to_line(state, &types_map);
            file.write_all(line.as_bytes()).unwrap();
        }

        file.write_all(b"\n").unwrap();

        let text = r#"
class StatesServer(StateServerBase):
    """The main class for the SteteServer for UI."""

    def __init__(
        self,
        port: int,
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        ip_addr: tuple[int, int, int, int] | None = None,
        handshake: list[int] | None = None,
    ) -> None:
        """Initialize the StateServer.

        Args:
            port (int): The port to listen on.
            signals_workers (int, optional): Number of workers for signal processing. Defaults to 3.
            error_handler (Callable[[Exception], None] | None, optional): Error handler function. Defaults to None.
            ip_addr (tuple[int, int, int, int] | None, optional): IP address to bind to. Defaults to None.
            handshake (list[int] | None, optional): Handshake bytes. Defaults to None.
        """
        "#;
        file.write_all(text.as_bytes()).unwrap();

        file.write_all(
            format!(
                "super().__init__({}, port, signals_workers, error_handler, ip_addr, handshake)\n",
                root_name
            )
            .as_bytes(),
        )
        .unwrap();
    } else {
        panic!("Root state must be a SubState");
    }

    Ok(())
}
