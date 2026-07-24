use std::collections::{HashMap, VecDeque};
use std::fmt::Write;
use std::path::Path;

use crate::InitValue;
use crate::State;
use crate::build_scripts::scripts;
use crate::build_scripts::states_creator_build::StateType;
use crate::data_transport::DataType;
use crate::typed::ObjectType;

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
        ObjectType::Enum(name, _) => format!("s.enu(enums.{name})"),
        ObjectType::Struct(name, elements) => {
            let fields = elements
                .iter()
                .map(|(_, object)| type_to_pytype(object))
                .collect::<Vec<_>>()
                .join(", ");
            format!("s.cl([{fields}], structs.{name})")
        }
        ObjectType::Tuple(elements) => {
            let elements = elements
                .iter()
                .map(type_to_pytype)
                .collect::<Vec<_>>()
                .join(", ");
            format!("s.tu([{elements}])")
        }
        ObjectType::List(size, element) => {
            format!("s.li({}, {size})", type_to_pytype(element))
        }
        ObjectType::Vec(element) => format!("s.vec({})", type_to_pytype(element)),
        ObjectType::Map(key, value) => {
            format!("s.map({}, {})", type_to_pytype(key), type_to_pytype(value))
        }
        ObjectType::Option(element) => format!("s.opt({})", type_to_pytype(element)),
    }
}

enum TypeIndex {
    Single(usize),
    Map(usize, usize),
}

impl TypeIndex {
    fn get_single(&self) -> usize {
        match self {
            TypeIndex::Single(index) => *index,
            TypeIndex::Map(_, _) => panic!("Expected single type index"),
        }
    }

    fn get_map(&self) -> (usize, usize) {
        match self {
            TypeIndex::Map(key, value) => (*key, *value),
            TypeIndex::Single(_) => panic!("Expected map type index"),
        }
    }
}

fn process_type_info(values: &[StateType]) -> (HashMap<String, TypeIndex>, Vec<ObjectType>) {
    let mut type_map = HashMap::new();
    let mut type_list = Vec::new();

    let mut type_index = |object_type: &ObjectType| {
        if let Some(index) = type_list.iter().position(|typ| typ == object_type) {
            index
        } else {
            type_list.push(object_type.clone());
            type_list.len() - 1
        }
    };

    for state in values {
        match state {
            StateType::Value(name, object_type, _, _)
            | StateType::ValueTake(name, object_type)
            | StateType::Static(name, object_type, _)
            | StateType::Signal(name, object_type, _)
            | StateType::ValueVec(name, object_type) => {
                let index = type_index(object_type);
                type_map.insert(name.clone(), TypeIndex::Single(index));
            }
            StateType::ValueMap(name, key, value) => {
                let key = type_index(key);
                let value = type_index(value);
                type_map.insert(name.clone(), TypeIndex::Map(key, value));
            }
            StateType::SubState(_, _, _)
            | StateType::Image(_)
            | StateType::Data(_, _)
            | StateType::DataTake(_, _)
            | StateType::DataMulti(_, _)
            | StateType::DataMultiTake(_, _) => {}
        }
    }

    (type_map, type_list)
}

#[derive(Clone, Copy)]
enum PythonTypeContext {
    States,
    Structs,
}

fn type_info_to_python_type(
    info: &ObjectType,
    list_comment: bool,
    context: PythonTypeContext,
) -> String {
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
        ObjectType::Empty => String::new(),
        ObjectType::Enum(name, _) => format!("enums.{name}"),
        ObjectType::Struct(name, _) => match context {
            PythonTypeContext::States => format!("structs.{name}"),
            PythonTypeContext::Structs => name.clone(),
        },
        ObjectType::Tuple(elements) => {
            let elements = elements
                .iter()
                .map(|element| type_info_to_python_type(element, list_comment, context))
                .collect::<Vec<_>>()
                .join(", ");
            format!("tuple[{elements}]")
        }
        ObjectType::List(size, element) => {
            let element = type_info_to_python_type(element, list_comment, context);
            if list_comment {
                format!("list[{element}]  # fixed size {size}")
            } else {
                format!("list[{element}]")
            }
        }
        ObjectType::Vec(element) => {
            let element = type_info_to_python_type(element, list_comment, context);
            format!("list[{element}]")
        }
        ObjectType::Map(key, value) => {
            let key = type_info_to_python_type(key, list_comment, context);
            let value = type_info_to_python_type(value, list_comment, context);
            format!("dict[{key}, {value}]")
        }
        ObjectType::Option(element) => {
            let element = type_info_to_python_type(element, list_comment, context);
            format!("{element} | None")
        }
    }
}

fn init_to_python_value(init: &InitValue, object_type: &ObjectType) -> String {
    match (init, object_type) {
        (InitValue::U8(value), ObjectType::U8) => value.to_string(),
        (InitValue::U16(value), ObjectType::U16) => value.to_string(),
        (InitValue::U32(value), ObjectType::U32) => value.to_string(),
        (InitValue::U64(value), ObjectType::U64) => value.to_string(),
        (InitValue::I8(value), ObjectType::I8) => value.to_string(),
        (InitValue::I16(value), ObjectType::I16) => value.to_string(),
        (InitValue::I32(value), ObjectType::I32) => value.to_string(),
        (InitValue::I64(value), ObjectType::I64) => value.to_string(),
        (InitValue::F64(value), ObjectType::F64) => value.to_string(),
        (InitValue::F32(value), ObjectType::F32) => value.to_string(),
        (InitValue::String(value), ObjectType::String) => format!("{value:?}"),
        (InitValue::Bool(value), ObjectType::Bool) => match value {
            true => "True".to_string(),
            false => "False".to_string(),
        },
        (InitValue::Enum(variant), ObjectType::Enum(name, _)) => {
            format!("enums.{name}.{variant}")
        }
        (InitValue::Option(value), ObjectType::Option(inner)) => match value {
            Some(value) => init_to_python_value(value, inner),
            None => "None".to_string(),
        },
        (InitValue::Tuple(values), ObjectType::Tuple(types)) => {
            let values = values
                .iter()
                .zip(types.iter())
                .map(|(value, typ)| init_to_python_value(value, typ))
                .collect::<Vec<_>>()
                .join(", ");
            if values.is_empty() {
                "()".to_string()
            } else if types.len() == 1 {
                format!("({values},)")
            } else {
                format!("({values})")
            }
        }
        (InitValue::List(values), ObjectType::List(_, element))
        | (InitValue::Vec(values), ObjectType::Vec(element)) => {
            let values = values
                .iter()
                .map(|value| init_to_python_value(value, element))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        }
        (InitValue::Map(values), ObjectType::Map(key, value)) => {
            let values = values
                .iter()
                .map(|(key_value, value_value)| {
                    format!(
                        "{}: {}",
                        init_to_python_value(key_value, key),
                        init_to_python_value(value_value, value)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{values}}}")
        }
        (InitValue::Struct(name, values), ObjectType::Struct(_, field_types)) => {
            let values = values
                .iter()
                .zip(field_types.iter())
                .map(|((_, value), (_, typ))| init_to_python_value(value, typ))
                .collect::<Vec<_>>()
                .join(", ");
            format!("structs.{name}({values})")
        }
        _ => panic!("Mismatched InitValue and ObjectType"),
    }
}

fn data_type_to_dtype(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::U8 => "np.uint8",
        DataType::U16 => "np.uint16",
        DataType::U32 => "np.uint32",
        DataType::U64 => "np.uint64",
        DataType::I8 => "np.int8",
        DataType::I16 => "np.int16",
        DataType::I32 => "np.int32",
        DataType::I64 => "np.int64",
        DataType::F32 => "np.float32",
        DataType::F64 => "np.float64",
    }
}

fn last_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn state_to_line(state: &StateType, types_map: &HashMap<String, TypeIndex>) -> String {
    match state {
        StateType::Value(name, state_type, init, queue) => {
            let name = last_name(name);
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let init_value = init_to_python_value(init, state_type);
            let index = types_map.get(state_name(state)).unwrap().get_single();
            let queue = if *queue { ", True" } else { "" };
            format!(
                "        self.{name}: s.Value[{py_type}] = s.Value[{py_type}]({index}, {init_value}{queue})\n"
            )
        }
        StateType::ValueTake(name, state_type) => {
            let full_name = name;
            let name = last_name(name);
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let index = types_map.get(full_name).unwrap().get_single();
            if matches!(state_type, ObjectType::Empty) {
                format!("        self.{name}: s.ValueTakeEmpty = s.ValueTakeEmpty()\n")
            } else {
                format!(
                    "        self.{name}: s.ValueTake[{py_type}] = s.ValueTake[{py_type}]({index})\n"
                )
            }
        }
        StateType::Static(name, state_type, init) => {
            let full_name = name;
            let name = last_name(name);
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let init_value = init_to_python_value(init, state_type);
            let index = types_map.get(full_name).unwrap().get_single();
            format!(
                "        self.{name}: s.Static[{py_type}] = s.Static[{py_type}]({index}, {init_value})\n"
            )
        }
        StateType::Signal(name, state_type, queue) => {
            let full_name = name;
            let name = last_name(name);
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let index = types_map.get(full_name).unwrap().get_single();
            if matches!(state_type, ObjectType::Empty) {
                let queue = if *queue { "True" } else { "" };
                format!("        self.{name}: s.SignalEmpty = s.SignalEmpty({queue})\n")
            } else {
                let queue = if *queue { ", True" } else { "" };
                format!(
                    "        self.{name}: s.Signal[{py_type}] = s.Signal[{py_type}]({index}{queue})\n"
                )
            }
        }
        StateType::ValueVec(name, state_type) => {
            let full_name = name;
            let name = last_name(name);
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let index = types_map.get(full_name).unwrap().get_single();
            format!("        self.{name}: s.Vec[{py_type}] = s.Vec[{py_type}]({index})\n")
        }
        StateType::ValueMap(name, key_type, value_type) => {
            let full_name = name;
            let name = last_name(name);
            let key_type = type_info_to_python_type(key_type, false, PythonTypeContext::States);
            let value_type = type_info_to_python_type(value_type, false, PythonTypeContext::States);
            let (key, value) = types_map.get(full_name).unwrap().get_map();
            format!(
                "        self.{name}: s.Map[{key_type}, {value_type}] = s.Map[{key_type}, {value_type}]({key}, {value})\n"
            )
        }
        StateType::Data(name, data_type) => {
            let name = last_name(name);
            let data_type = data_type_to_dtype(data_type);
            format!("        self.{name}: s.Data[{data_type}] = s.Data({data_type})\n")
        }
        StateType::DataMulti(name, data_type) => {
            let name = last_name(name);
            let data_type = data_type_to_dtype(data_type);
            format!("        self.{name}: s.DataMulti[{data_type}] = s.DataMulti({data_type})\n")
        }
        StateType::DataTake(name, data_type) => {
            let name = last_name(name);
            let data_type = data_type_to_dtype(data_type);
            format!("        self.{name}: s.DataTake[{data_type}] = s.DataTake({data_type})\n")
        }
        StateType::DataMultiTake(name, data_type) => {
            let name = last_name(name);
            let data_type = data_type_to_dtype(data_type);
            format!(
                "        self.{name}: s.DataMultiTake[{data_type}] = s.DataMultiTake({data_type})\n"
            )
        }
        StateType::Image(name) => {
            let name = last_name(name);
            format!("        self.{name}: s.Image = s.Image()\n")
        }
        StateType::SubState(name, state_class, _) => {
            let name = last_name(name);
            format!("        self.{name}: {state_class} = {state_class}(parent + \".{name}\")\n")
        }
    }
}

fn state_name(state: &StateType) -> &str {
    match state {
        StateType::Value(name, _, _, _)
        | StateType::ValueTake(name, _)
        | StateType::Static(name, _, _)
        | StateType::Image(name)
        | StateType::ValueMap(name, _, _)
        | StateType::ValueVec(name, _)
        | StateType::Signal(name, _, _)
        | StateType::Data(name, _)
        | StateType::DataTake(name, _)
        | StateType::DataMulti(name, _)
        | StateType::DataMultiTake(name, _)
        | StateType::SubState(name, _, _) => name,
    }
}

fn write_states(
    output: &mut String,
    state_class: &str,
    states: &[StateType],
    types_map: &HashMap<String, TypeIndex>,
    used_states: &mut Vec<String>,
) {
    for state in states {
        if let StateType::SubState(_, state_class, sub_states) = state {
            if used_states.iter().any(|used| used == state_class) {
                continue;
            }
            used_states.push(state_class.to_string());
            write_states(output, state_class, sub_states, types_map, used_states);
        }
    }

    writeln!(
        output,
        "\n\nclass {state_class}(ISubStates):\n    def __init__(self, parent: str):"
    )
    .unwrap();
    if states.is_empty() {
        output.push_str("        pass\n");
    } else {
        for state in states {
            output.push_str(&state_to_line(state, types_map));
        }
    }
}

fn order_structs(items: &[(String, ObjectType)], order: &mut VecDeque<String>) {
    for (_, item_type) in items {
        if let ObjectType::Struct(name, fields) = item_type
            && !order.contains(name)
        {
            order.push_front(name.clone());
            order_structs(fields, order);
        }
    }
}

fn render_python<S: State>() -> (String, String, String) {
    let (states, version_hash) = scripts::parse_states::<S>();
    scripts::validate_states(&states);

    let mut values_list = Vec::new();
    scripts::states_into_values_list(&states, &mut values_list);
    let (enums, structs) = scripts::get_all_enums_struct(&values_list);
    let (types_map, types_list) = process_type_info(&values_list);

    let mut struct_order = VecDeque::new();
    for (struct_name, items) in &structs {
        if !struct_order.contains(struct_name) {
            struct_order.push_front(struct_name.clone());
            order_structs(items, &mut struct_order);
        }
    }

    let mut enums_output = String::from(
        "# Generated by build.rs, do not edit\n# ruff: noqa: D101\nfrom enum import IntEnum\n",
    );
    for (enum_name, variants) in &enums {
        writeln!(enums_output, "\n\nclass {enum_name}(IntEnum):").unwrap();
        for (variant, value) in variants {
            writeln!(enums_output, "    {variant} = {value}").unwrap();
        }
    }

    let mut structs_output = String::from(
        "# Generated by build.rs, do not edit\n# ruff: noqa: D101 E501 F401\nfrom __future__ import annotations\n\nfrom dataclasses import dataclass\n\nimport egui_states.structures as s\n\nfrom . import enums\n",
    );
    for struct_name in &struct_order {
        let fields = &structs[struct_name];
        writeln!(
            structs_output,
            "\n\n@dataclass\nclass {struct_name}(s._CustomStruct):"
        )
        .unwrap();
        if fields.is_empty() {
            structs_output.push_str("    pass\n");
        } else {
            for (field, typ) in fields {
                let typ = type_info_to_python_type(typ, true, PythonTypeContext::Structs);
                writeln!(structs_output, "    {field}: {typ}").unwrap();
            }
        }
    }

    let mut states_output = String::from(
        "# Generated by build.rs, do not edit\n# ruff: noqa: D101 D107 E501\nfrom __future__ import annotations\n\nfrom collections.abc import Callable\n\nimport numpy as np\n\nimport egui_states.structures as s\nfrom egui_states.server import StatesBase, StateServerBase\nfrom egui_states.structures import ISubStates\n\nfrom . import enums, structs\n",
    );

    let StateType::SubState(_, root_name, substates) = &states else {
        panic!("Root state must be a SubState");
    };

    let mut used_states = Vec::new();
    for state in substates {
        if let StateType::SubState(_, state_class, sub_states) = state {
            if used_states.iter().any(|used| used == state_class) {
                continue;
            }
            used_states.push(state_class.to_string());
            write_states(
                &mut states_output,
                state_class,
                sub_states,
                &types_map,
                &mut used_states,
            );
        }
    }

    writeln!(
        states_output,
        "\n\nclass {root_name}(StatesBase):\n    @staticmethod\n    def _get_obj_types() -> list[s.PyObjectType]:\n        return ["
    )
    .unwrap();
    for object_type in &types_list {
        writeln!(
            states_output,
            "            {},",
            type_to_pytype(object_type)
        )
        .unwrap();
    }
    states_output.push_str(
        "        ]\n\n    def __init__(self, server: StateServerBase):\n        super().__init__(server)\n        parent = \"root\"\n",
    );
    for state in substates {
        states_output.push_str(&state_to_line(state, &types_map));
    }

    write!(
        states_output,
        r#"

class StatesServer(StateServerBase[{root_name}]):
    """The main class for the StateServer for UI."""

    VERSION_HASH: int = {version_hash}

    def __init__(
        self,
        port: int,
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        ip_addr: tuple[int, int, int, int] | None = None,
        version: int | None = None,
        token: str | None = None,
    ) -> None:
        """Initialize the StateServer.

        Args:
            port (int): The port to listen on.
            signals_workers (int, optional): Number of workers for signal processing. Defaults to 3.
            error_handler (Callable[[Exception], None] | None, optional): Error handler function. Defaults to None.
            ip_addr (tuple[int, int, int, int] | None, optional): IP address to bind to. Defaults to None.
            version (int, optional): The optional version number for client connection.
            token (str, optional): The optional token string for client connection.
        """
        super().__init__({root_name}, port, signals_workers, error_handler, ip_addr, version, token)
"#
    )
    .unwrap();

    (states_output, enums_output, structs_output)
}

/// Generates typed Python server bindings as a three-file package.
pub fn generate_python<S: State>(directory: impl AsRef<Path>) -> Result<(), String> {
    let (states, enums, structs) = render_python::<S>();
    scripts::write_generated_files(
        directory.as_ref(),
        [
            ("__init__.py", states.as_str()),
            ("enums.py", enums.as_str()),
            ("structs.py", structs.as_str()),
        ],
    )
}
