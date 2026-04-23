use std::collections::HashMap;
use std::collections::VecDeque;
use std::string::ToString;
use std::{fs, io::Write};

use crate::State;
use crate::build_scripts::scripts;
use crate::build_scripts::state_creator::StateType;
use crate::data_transport::DataType;
use crate::transport::{InitValue, ObjectType};

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

enum TypeIndex {
    Single(usize),
    Map(usize, usize),
}

impl TypeIndex {
    fn get_single(&self) -> usize {
        match self {
            TypeIndex::Single(i) => *i,
            _ => panic!("Expected single type index"),
        }
    }
    fn get_map(&self) -> (usize, usize) {
        match self {
            TypeIndex::Map(k, v) => (*k, *v),
            _ => panic!("Expected map type index"),
        }
    }
}

fn process_type_info(values: &Vec<StateType>) -> (HashMap<String, TypeIndex>, Vec<ObjectType>) {
    let mut type_map: HashMap<String, TypeIndex> = HashMap::new();
    let mut type_list: Vec<ObjectType> = Vec::new();

    for state in values {
        match state {
            StateType::Value(name, obj_type, _, _)
            | StateType::ValueTake(name, obj_type)
            | StateType::Static(name, obj_type, _)
            | StateType::Signal(name, obj_type, _)
            | StateType::ValueVec(name, obj_type) => {
                if type_list.contains(obj_type) {
                    type_map.insert(
                        name.clone(),
                        TypeIndex::Single(type_list.iter().position(|t| t == obj_type).unwrap()),
                    );
                } else {
                    type_list.push(obj_type.clone());
                    type_map.insert(name.clone(), TypeIndex::Single(type_list.len() - 1));
                }
            }
            StateType::ValueMap(name, key, value) => {
                // let dict_type = ObjectType::Map(Box::new(key.clone()), Box::new(value.clone()));
                let key_pos = if type_list.contains(key) {
                    type_list.iter().position(|t| t == key).unwrap()
                } else {
                    type_list.push(key.clone());
                    type_list.len() - 1
                };

                let value_pos = if type_list.contains(value) {
                    type_list.iter().position(|t| t == value).unwrap()
                } else {
                    type_list.push(value.clone());
                    type_list.len() - 1
                };
                type_map.insert(name.clone(), TypeIndex::Map(key_pos, value_pos));
            }
            StateType::SubState(_, _, _)
            | StateType::Image(_)
            | StateType::Data(_, _) => {}
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

fn init_to_python_value(init: &InitValue, object_type: &ObjectType) -> String {
    match (init, object_type) {
        (InitValue::U8(v), ObjectType::U8) => format!("{}", v),
        (InitValue::U16(v), ObjectType::U16) => format!("{}", v),
        (InitValue::U32(v), ObjectType::U32) => format!("{}", v),
        (InitValue::U64(v), ObjectType::U64) => format!("{}", v),
        (InitValue::I8(v), ObjectType::I8) => format!("{}", v),
        (InitValue::I16(v), ObjectType::I16) => format!("{}", v),
        (InitValue::I32(v), ObjectType::I32) => format!("{}", v),
        (InitValue::I64(v), ObjectType::I64) => format!("{}", v),
        (InitValue::F64(v), ObjectType::F64) => format!("{}", v),
        (InitValue::F32(v), ObjectType::F32) => format!("{}", v),
        (InitValue::String(v), ObjectType::String) => format!("\"{}\"", v),
        (InitValue::Bool(v), ObjectType::Bool) => match v {
            true => "True".to_string(),
            false => "False".to_string(),
        },
        (InitValue::Enum(v), ObjectType::Enum(name, _)) => {
            format!("{}.{}", name, v)
        }
        (InitValue::Option(opt), ObjectType::Option(inner)) => match opt {
            Some(boxed) => init_to_python_value(boxed, inner),
            None => "None".to_string(),
        },
        (InitValue::Tuple(elems), ObjectType::Tuple(types)) => {
            let elem_strs: Vec<String> = elems
                .iter()
                .zip(types.iter())
                .map(|(e, t)| init_to_python_value(e, t))
                .collect();
            format!("({})", elem_strs.join(", "))
        }
        (InitValue::List(elems), ObjectType::List(_, element))
        | (InitValue::Vec(elems), ObjectType::Vec(element)) => {
            let elem_strs: Vec<String> = elems
                .iter()
                .map(|e| init_to_python_value(e, element))
                .collect();
            format!("[{}]", elem_strs.join(", "))
        }
        (InitValue::Map(pairs), ObjectType::Map(key, value)) => {
            let pair_strs: Vec<String> = pairs
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        init_to_python_value(k, key),
                        init_to_python_value(v, value)
                    )
                })
                .collect();
            format!("{{{}}}", pair_strs.join(", "))
        }
        (InitValue::Struct(name, items), ObjectType::Struct(_, field_types)) => {
            let field_strs: Vec<String> = items
                .iter()
                .zip(field_types.iter())
                .map(|((_, value), (_, field_type))| init_to_python_value(value, field_type))
                .collect();
            format!("{}({})", name, field_strs.join(", "))
        }
        _ => panic!("Mismatched InitValue and ObjectType."),
    }
}

fn state_to_line(state: &StateType, types_map: &HashMap<String, TypeIndex>) -> String {
    match state {
        StateType::Value(name, state_type, init, queue) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let init_value = init_to_python_value(init, state_type);
            let index = types_map.get(name).unwrap().get_single();
            let queue_str = match queue {
                true => ", True",
                false => "",
            };
            format!(
                "        self.{}: s.Value[{}] = s.Value[{}]({}, {}{})\n",
                last_name, py_type, py_type, index, init_value, queue_str
            )
        }
        StateType::ValueTake(name, state_type) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let index = types_map.get(name).unwrap().get_single();
            match state_type {
                ObjectType::Empty => {
                    format!(
                        "        self.{}: s.ValueTakeEmpty = s.ValueTakeEmpty()\n",
                        last_name
                    )
                }
                _ => format!(
                    "        self.{}: s.ValueTake[{}] = s.ValueTake[{}]({})\n",
                    last_name, py_type, py_type, index
                ),
            }
        }
        StateType::Static(name, state_type, init) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let init_value = init_to_python_value(init, state_type);
            let index = types_map.get(name).unwrap().get_single();
            format!(
                "        self.{}: s.Static[{}] = s.Static[{}]({}, {})\n",
                last_name, py_type, py_type, index, init_value
            )
        }
        StateType::Signal(name, state_type, queue) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let index = types_map.get(name).unwrap().get_single();
            match state_type {
                ObjectType::Empty => {
                    let queue_str = match queue {
                        true => "True",
                        false => "",
                    };
                    format!(
                        "        self.{}: s.SignalEmpty = s.SignalEmpty({})\n",
                        last_name, queue_str
                    )
                }
                _ => {
                    let queue_str = match queue {
                        true => ", True",
                        false => "",
                    };
                    format!(
                        "        self.{}: s.Signal[{}] = s.Signal[{}]({}{})\n",
                        last_name, py_type, py_type, index, queue_str
                    )
                }
            }
        }
        StateType::ValueVec(name, state_type) => {
            let last_name = name.split('.').last().unwrap();
            let py_type = type_info_to_python_type(state_type, false);
            let index = types_map.get(name).unwrap().get_single();
            format!(
                "        self.{}: s.ValueVec[{}] = s.ValueVec[{}]({})\n",
                last_name, py_type, py_type, index
            )
        }
        StateType::ValueMap(name, key_type, value_type) => {
            let last_name = name.split('.').last().unwrap();
            let py_key_type = type_info_to_python_type(key_type, false);
            let py_value_type = type_info_to_python_type(value_type, false);
            let (key, value) = types_map.get(name).unwrap().get_map();
            format!(
                "        self.{}: s.ValueMap[{}, {}] = s.ValueMap[{}, {}]({}, {})\n",
                last_name, py_key_type, py_value_type, py_key_type, py_value_type, key, value
            )
        }
        StateType::Data(name, data_type) => {
            let last_name = name.split('.').last().unwrap();
            let data_id = data_type.get_id();
            let dtype = match data_type {
                DataType::U8 => "uint8",
                DataType::U16 => "uint16",
                DataType::U32 => "uint32",
                DataType::U64 => "uint64",
                DataType::I8 => "int8",
                DataType::I16 => "int16",
                DataType::I32 => "int32",
                DataType::I64 => "int64",
                DataType::F32 => "float32",
                DataType::F64 => "float64",
            };
            let dtype = format!("np.{}", dtype);
            format!(
                "        self.{}: s.Data[{}] = s.Data[{}]({})\n",
                last_name, dtype, dtype, data_id
            )
        }
        StateType::Image(name) => {
            let last_name = name.split('.').last().unwrap();
            format!(
                "        self.{}: s.ValueImage = s.ValueImage()\n",
                last_name
            )
        }
        StateType::SubState(name, state_class, _) => {
            let last_name = name.split('.').last().unwrap();
            format!(
                "        self.{}: {} = {}(parent + \".{}\")\n",
                last_name, state_class, state_class, last_name
            )
        }
    }
}

fn write_states(
    file: &mut fs::File,
    state_class: &str,
    states: &Vec<StateType>,
    types_map: &HashMap<String, TypeIndex>,
    used_states: &mut Vec<&str>,
) {
    let mut lines = Vec::new();

    for state in states {
        lines.push(state_to_line(state, types_map));
        if let StateType::SubState(_, state_class, sub_states) = state {
            if used_states.contains(state_class) {
                continue;
            }
            used_states.push(state_class);
            write_states(file, state_class, sub_states, types_map, used_states);
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

pub fn generate_python<S: State>(path: impl ToString) -> Result<(), String> {
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
    if enums.len() > 0 {
        file.write_all(b"from enum import IntEnum\n").unwrap();
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
        file.write_all(format!("\n\nclass {}(IntEnum):\n", enum_name).as_bytes())
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
        let mut used_states = Vec::new();
        for state in substates {
            if let StateType::SubState(_, state_class, sub_states) = state {
                if used_states.contains(state_class) {
                    continue;
                }
                used_states.push(state_class);
                write_states(
                    &mut file,
                    state_class,
                    sub_states,
                    &types_map,
                    &mut used_states,
                );
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
    """The main class for the StateServer for UI.""""#;
        file.write_all(text.as_bytes()).unwrap();

        file.write_all(format!("\n\n    states: {}\n", root_name).as_bytes())
            .unwrap();

        let text = r#"
    def __init__(
        self,
        port: int,
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        ip_addr: tuple[int, int, int, int] | None = None,
        handshake: list[int] | None = None,
        runner_threads: int = 3,
    ) -> None:
        """Initialize the StateServer.

        Args:
            port (int): The port to listen on.
            signals_workers (int, optional): Number of workers for signal processing. Defaults to 3.
            error_handler (Callable[[Exception], None] | None, optional): Error handler function. Defaults to None.
            ip_addr (tuple[int, int, int, int] | None, optional): IP address to bind to. Defaults to None.
            handshake (list[int] | None, optional): Handshake bytes. Defaults to None.
            runner_threads (int): The number of threads for running the server.
        """
        "#;
        file.write_all(text.as_bytes()).unwrap();

        file.write_all(
            format!(
                "super().__init__({}, port, signals_workers, error_handler, ip_addr, handshake, runner_threads)\n",
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
