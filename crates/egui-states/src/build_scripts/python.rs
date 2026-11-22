use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::Write;
use std::path::Path;

use crate::InitValue;
use crate::State;
use crate::build_scripts::scripts;
use crate::build_scripts::states_creator_build::StateType;
use crate::protocol::data::DataType;
use crate::typed::{ObjectType, VariantFields};

/// Python's hard keywords. Soft keywords (`match`, `case`, `type`, `_`) are valid
/// identifiers and need no escaping.
const PYTHON_KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Spells a Rust name as a Python identifier, escaping keywords with a trailing
/// underscore (PEP 8).
fn python_name(name: &str) -> Cow<'_, str> {
    if PYTHON_KEYWORDS.contains(&name) {
        Cow::Owned(format!("{name}_"))
    } else {
        Cow::Borrowed(name)
    }
}

/// The name every enum with data gives the union of its variants, as in
/// `Command.Type`.
const VARIANTS_ALIAS: &str = "Type";

/// Spells a variant of an enum with data as a Python class name. Besides the
/// keywords of [`python_name`], a variant called `Type` gets an underscore,
/// because the enum uses that name for the union of its variants.
fn variant_name(name: &str) -> Cow<'_, str> {
    if name == VARIANTS_ALIAS {
        Cow::Owned(format!("{name}_"))
    } else {
        python_name(name)
    }
}

/// Returns the `__egui_names__` class attribute that maps escaped Python names
/// back to the names used on the wire (state paths and type hashes), or an empty
/// string when no name of `owner` was escaped.
///
/// # Panics
///
/// Panics if an escaped name collides with another name of `owner`.
fn wire_names_line<'a>(owner: &str, names: impl IntoIterator<Item = &'a str> + Clone) -> String {
    let mut escaped = Vec::new();
    for name in names.clone() {
        if let Cow::Owned(python) = python_name(name) {
            if names.clone().into_iter().any(|other| other == python) {
                panic!(
                    "{owner}: the Python name `{python}` of `{name}` conflicts with another name"
                );
            }
            escaped.push(format!("\"{python}\": \"{name}\""));
        }
    }
    if escaped.is_empty() {
        String::new()
    } else {
        format!("    __egui_names__ = {{{}}}\n", escaped.join(", "))
    }
}

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
        ObjectType::DataEnum(name, variants) => {
            let variants = variants
                .iter()
                .map(|(variant, fields)| {
                    let attr = format!("enums.{name}.{}", variant_name(variant));
                    match fields {
                        VariantFields::Unit => format!("s.vunit(\"{variant}\", {attr})"),
                        VariantFields::Named(fields) => {
                            let fields = fields
                                .iter()
                                .map(|(_, object)| type_to_pytype(object))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("s.vnamed(\"{variant}\", [{fields}], {attr})")
                        }
                        VariantFields::Tuple(types) => {
                            let types = types
                                .iter()
                                .map(type_to_pytype)
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("s.vtuple(\"{variant}\", [{types}], {attr})")
                        }
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("s.denu(enums.{name}, [{variants}])")
        }
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

/// The attribute of a Data state of kind `kind`; with a header, the
/// `WithHeader` class of the kind.
fn data_state(
    attr: &str,
    kind: &str,
    data_type: &DataType,
    header: Option<&ObjectType>,
    priority: Priority,
    object_types: &[ObjectType],
) -> String {
    let data_type = data_type_to_dtype(data_type);
    let priority = priority.argument(", ");
    match header {
        None => format!(
            "        self.{attr}: s.{kind}[{data_type}] = s.{kind}({data_type}{priority})\n"
        ),
        Some(header) => {
            let index = object_type_index(object_types, header);
            let py_type = type_info_to_python_type(header, false, PythonTypeContext::States);
            format!(
                "        self.{attr}: s.{kind}WithHeader[{data_type}, {py_type}] = s.{kind}WithHeader({data_type}, {index}{priority})\n"
            )
        }
    }
}

/// The attribute of an image state of kind `kind`; with a header, the
/// `WithHeader` class of the kind.
fn image_state(
    attr: &str,
    kind: &str,
    header: Option<&ObjectType>,
    priority: Priority,
    object_types: &[ObjectType],
) -> String {
    match header {
        None => format!(
            "        self.{attr}: s.{kind} = s.{kind}({})\n",
            priority.argument("")
        ),
        Some(header) => {
            let index = object_type_index(object_types, header);
            let py_type = type_info_to_python_type(header, false, PythonTypeContext::States);
            let priority = priority.argument(", ");
            format!(
                "        self.{attr}: s.{kind}WithHeader[{py_type}] = s.{kind}WithHeader({index}{priority})\n"
            )
        }
    }
}

/// Where the send priority of a bulk state or substate field comes from.
#[derive(Clone, Copy)]
enum Priority {
    /// The field is marked.
    Low,
    /// The `low_priority` argument of the enclosing substate class.
    Inherited,
    /// The default, in the root class.
    Normal,
}

impl Priority {
    /// Of a field marked `low` or not, in a substate class (`inherits`) or
    /// in the root class.
    fn new(low: bool, inherits: bool) -> Self {
        match (low, inherits) {
            (true, _) => Self::Low,
            (false, true) => Self::Inherited,
            (false, false) => Self::Normal,
        }
    }

    /// The keyword argument after `separator`; nothing for the default, so
    /// it stays implicit.
    fn argument(self, separator: &str) -> String {
        match self {
            Self::Low => format!("{separator}low_priority=True"),
            Self::Inherited => format!("{separator}low_priority=low_priority"),
            Self::Normal => String::new(),
        }
    }
}

/// The attribute of a transfer endpoint of kind `kind`; with metadata, the
/// `WithMeta` class of the kind.
fn transfer_state(
    attr: &str,
    kind: &str,
    meta: Option<&ObjectType>,
    object_types: &[ObjectType],
) -> String {
    match meta {
        None => format!("        self.{attr}: s.{kind} = s.{kind}()\n"),
        Some(meta) => {
            let index = object_type_index(object_types, meta);
            let py_type = type_info_to_python_type(meta, false, PythonTypeContext::States);
            format!(
                "        self.{attr}: s.{kind}WithMeta[{py_type}] = s.{kind}WithMeta({index})\n"
            )
        }
    }
}

fn collect_object_types(values: &[StateType]) -> Vec<ObjectType> {
    let mut type_list = Vec::new();

    let mut push_type = |object_type: &ObjectType| {
        if !type_list.contains(object_type) {
            type_list.push(object_type.clone());
        }
    };

    for state in values {
        match state {
            StateType::Value(_, object_type, _, _)
            | StateType::ValueTake(_, object_type)
            | StateType::Static(_, object_type, _)
            | StateType::Signal(_, object_type, _)
            | StateType::ValueVec(_, object_type) => {
                push_type(object_type);
            }
            StateType::ValueMap(_, key, value) => {
                push_type(key);
                push_type(value);
            }
            StateType::Data(_, _, Some(header), _)
            | StateType::DataTake(_, _, Some(header), _)
            | StateType::DataMulti(_, _, Some(header), _)
            | StateType::DataMultiTake(_, _, Some(header), _)
            | StateType::Download(_, Some(header))
            | StateType::Upload(_, Some(header))
            | StateType::Image(_, Some(header), _)
            | StateType::ImageMulti(_, Some(header), _) => push_type(header),
            StateType::SubState(..)
            | StateType::Image(..)
            | StateType::ImageMulti(..)
            | StateType::Data(..)
            | StateType::DataTake(..)
            | StateType::DataMulti(..)
            | StateType::DataMultiTake(..)
            | StateType::Download(..)
            | StateType::Upload(..) => {}
        }
    }

    type_list
}

fn object_type_index(types: &[ObjectType], object_type: &ObjectType) -> usize {
    types
        .iter()
        .position(|typ| typ == object_type)
        .expect("State object type missing from generated type list")
}

#[derive(Clone, Copy)]
enum PythonTypeContext {
    States,
    Structs,
    /// The `enums.py` module itself, next to the enums.
    Enums,
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
        ObjectType::Enum(name, _) => match context {
            PythonTypeContext::States | PythonTypeContext::Structs => format!("enums.{name}"),
            PythonTypeContext::Enums => name.clone(),
        },
        // The class of an enum with data is a namespace; a value is one of
        // its variants.
        ObjectType::DataEnum(name, _) => match context {
            PythonTypeContext::States | PythonTypeContext::Structs => {
                format!("enums.{name}.{VARIANTS_ALIAS}")
            }
            PythonTypeContext::Enums => format!("{name}.{VARIANTS_ALIAS}"),
        },
        ObjectType::Struct(name, _) => match context {
            PythonTypeContext::States | PythonTypeContext::Enums => format!("structs.{name}"),
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

/// Python has no literals for non-finite floats; `text` is used for finite values.
fn python_float(text: String, value: f64) -> String {
    if value.is_nan() {
        "float(\"nan\")".to_string()
    } else if value == f64::INFINITY {
        "float(\"inf\")".to_string()
    } else if value == f64::NEG_INFINITY {
        "-float(\"inf\")".to_string()
    } else {
        text
    }
}

/// Rust's `Debug` escapes (`\u{1}`) are not valid Python, so escape for Python.
fn python_string(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for c in value.chars() {
        match c {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            // Control characters are all below U+00A0.
            c if c.is_control() => literal.push_str(&format!("\\x{:02x}", c as u32)),
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
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
        (InitValue::F64(value), ObjectType::F64) => python_float(value.to_string(), *value),
        (InitValue::F32(value), ObjectType::F32) => python_float(value.to_string(), *value as f64),
        (InitValue::String(value), ObjectType::String) => python_string(value),
        (InitValue::Bool(value), ObjectType::Bool) => match value {
            true => "True".to_string(),
            false => "False".to_string(),
        },
        (InitValue::Enum(variant), ObjectType::Enum(name, _)) => {
            format!("enums.{name}.{}", python_name(variant))
        }
        (InitValue::DataEnum(variant, values), ObjectType::DataEnum(name, variants)) => {
            let fields = variants
                .iter()
                .find(|(known, _)| known == variant)
                .map(|(_, fields)| fields)
                .expect("Mismatched InitValue and ObjectType");
            let attr = format!("enums.{name}.{}", variant_name(variant));
            let types: Vec<&ObjectType> = match fields {
                VariantFields::Unit => return format!("{attr}()"),
                VariantFields::Named(fields) => fields.iter().map(|(_, typ)| typ).collect(),
                VariantFields::Tuple(types) => types.iter().collect(),
            };
            let values = values
                .iter()
                .zip(types)
                .map(|(value, typ)| init_to_python_value(value, typ))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{attr}({values})")
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

/// The line that creates `state`; `inherits` in a substate class, whose bulk
/// states take the class's `low_priority` argument unless they are marked.
fn state_to_line(state: &StateType, object_types: &[ObjectType], inherits: bool) -> String {
    let attr = python_name(state.name());
    match state {
        StateType::Value(_, state_type, init, queue) => {
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let init_value = init_to_python_value(init, state_type);
            let index = object_type_index(object_types, state_type);
            let queue = if *queue { ", True" } else { "" };
            format!(
                "        self.{attr}: s.Value[{py_type}] = s.Value[{py_type}]({index}, {init_value}{queue})\n"
            )
        }
        StateType::ValueTake(_, state_type) => {
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            if matches!(state_type, ObjectType::Empty) {
                format!("        self.{attr}: s.ValueTakeEmpty = s.ValueTakeEmpty()\n")
            } else {
                let index = object_type_index(object_types, state_type);
                format!(
                    "        self.{attr}: s.ValueTake[{py_type}] = s.ValueTake[{py_type}]({index})\n"
                )
            }
        }
        StateType::Static(_, state_type, init) => {
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let init_value = init_to_python_value(init, state_type);
            let index = object_type_index(object_types, state_type);
            format!(
                "        self.{attr}: s.Static[{py_type}] = s.Static[{py_type}]({index}, {init_value})\n"
            )
        }
        StateType::Signal(_, state_type, queue) => {
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            if matches!(state_type, ObjectType::Empty) {
                let queue = if *queue { "True" } else { "" };
                format!("        self.{attr}: s.SignalEmpty = s.SignalEmpty({queue})\n")
            } else {
                let index = object_type_index(object_types, state_type);
                let queue = if *queue { ", True" } else { "" };
                format!(
                    "        self.{attr}: s.Signal[{py_type}] = s.Signal[{py_type}]({index}{queue})\n"
                )
            }
        }
        StateType::ValueVec(_, state_type) => {
            let py_type = type_info_to_python_type(state_type, false, PythonTypeContext::States);
            let index = object_type_index(object_types, state_type);
            format!("        self.{attr}: s.Vec[{py_type}] = s.Vec[{py_type}]({index})\n")
        }
        StateType::ValueMap(_, key_type, value_type) => {
            let key = object_type_index(object_types, key_type);
            let value = object_type_index(object_types, value_type);
            let key_type = type_info_to_python_type(key_type, false, PythonTypeContext::States);
            let value_type = type_info_to_python_type(value_type, false, PythonTypeContext::States);
            format!(
                "        self.{attr}: s.Map[{key_type}, {value_type}] = s.Map[{key_type}, {value_type}]({key}, {value})\n"
            )
        }
        StateType::Data(_, data_type, header, low) => data_state(
            &attr,
            "Data",
            data_type,
            header.as_ref(),
            Priority::new(*low, inherits),
            object_types,
        ),
        StateType::DataMulti(_, data_type, header, low) => data_state(
            &attr,
            "DataMulti",
            data_type,
            header.as_ref(),
            Priority::new(*low, inherits),
            object_types,
        ),
        StateType::DataTake(_, data_type, header, low) => data_state(
            &attr,
            "DataTake",
            data_type,
            header.as_ref(),
            Priority::new(*low, inherits),
            object_types,
        ),
        StateType::DataMultiTake(_, data_type, header, low) => data_state(
            &attr,
            "DataMultiTake",
            data_type,
            header.as_ref(),
            Priority::new(*low, inherits),
            object_types,
        ),
        StateType::Download(_, meta) => {
            transfer_state(&attr, "Download", meta.as_ref(), object_types)
        }
        StateType::Upload(_, meta) => transfer_state(&attr, "Upload", meta.as_ref(), object_types),
        StateType::Image(_, header, low) => image_state(
            &attr,
            "Image",
            header.as_ref(),
            Priority::new(*low, inherits),
            object_types,
        ),
        StateType::ImageMulti(_, header, low) => image_state(
            &attr,
            "ImageMulti",
            header.as_ref(),
            Priority::new(*low, inherits),
            object_types,
        ),
        StateType::SubState(name, state_class, _, low) => {
            // Only a class with bulk states takes the priority.
            let priority = match state.takes_priority() {
                true => Priority::new(*low, inherits).argument(", "),
                false => String::new(),
            };
            format!(
                "        self.{attr}: {state_class} = {state_class}(parent + \".{name}\"{priority})\n"
            )
        }
    }
}

fn write_states(
    output: &mut String,
    state_class: &str,
    states: &[StateType],
    object_types: &[ObjectType],
    used_states: &mut Vec<String>,
) {
    for state in states {
        if let StateType::SubState(_, state_class, sub_states, _) = state {
            if used_states.iter().any(|used| used == state_class) {
                continue;
            }
            used_states.push(state_class.to_string());
            write_states(output, state_class, sub_states, object_types, used_states);
        }
    }

    let mut wire_names = wire_names_line(state_class, states.iter().map(StateType::name));
    if !wire_names.is_empty() {
        wire_names.push('\n');
    }
    // A group with images or buffers takes the priority of the field it is
    // created for, so one class serves fields of either priority.
    let inherits = states.iter().any(StateType::takes_priority);
    let (parameter, argument) = match inherits {
        true => (
            ", low_priority: bool = False",
            "            low_priority (bool, optional): Whether the images and buffers\n                of the group use the low-priority lane. Defaults to False.\n",
        ),
        false => ("", ""),
    };
    writeln!(
        output,
        "\n\nclass {state_class}(ISubStates):\n    \"\"\"Generated handles for a nested state group.\"\"\"\n\n{wire_names}    def __init__(self, parent: str{parameter}) -> None:\n        \"\"\"Initialize handles below a generated state path.\n\n        Args:\n            parent (str): Fully qualified parent state path.\n{argument}        \"\"\""
    )
    .unwrap();
    if states.is_empty() {
        output.push_str("        pass\n");
    } else {
        for state in states {
            output.push_str(&state_to_line(state, object_types, inherits));
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

/// The fields of a variant as `(Python name, Python type)`, with the
/// positional names `_0`, `_1`, ... of a tuple variant.
fn variant_fields(fields: &VariantFields) -> Vec<(String, String)> {
    let python_type = |typ| type_info_to_python_type(typ, false, PythonTypeContext::Enums);
    match fields {
        VariantFields::Unit => Vec::new(),
        VariantFields::Named(fields) => fields
            .iter()
            .map(|(field, typ)| (python_name(field).into_owned(), python_type(typ)))
            .collect(),
        VariantFields::Tuple(types) => types
            .iter()
            .enumerate()
            .map(|(index, typ)| (format!("_{index}"), python_type(typ)))
            .collect(),
    }
}

/// Renders `enums.py`. A fieldless enum is an `IntEnum`. An enum with data is
/// a namespace class: each variant is a frozen dataclass nested in it, and
/// its `Type` alias is the union of them, the type of a value.
///
/// # Panics
///
/// Panics if two variants of an enum, or two fields of a variant, get the
/// same Python name.
fn render_enums(enums: &scripts::Enums, data_enums: &scripts::DataEnums) -> String {
    let header = "# Generated by build.rs, do not edit\n";
    let docstring = "\"\"\"Custom enum types used by the generated state server.\"\"\"\n";
    let mut module = if data_enums.is_empty() {
        format!("{header}{docstring}\nfrom enum import IntEnum\n")
    } else {
        // Annotations stay unevaluated, so a variant can name an enum defined
        // below it and `structs`, which imports this module.
        format!(
            "{header}# ruff: noqa: E501 F401\n{docstring}\nfrom __future__ import annotations\n\nfrom dataclasses import dataclass\nfrom enum import IntEnum\nfrom typing import final\n\nimport egui_states.structures as s\n\nfrom . import structs\n"
        )
    };

    for (enum_name, variants) in enums {
        writeln!(
            module,
            "\n\nclass {enum_name}(IntEnum):\n    \"\"\"A generated protocol enumeration.\"\"\""
        )
        .unwrap();
        for (variant, value) in variants {
            writeln!(module, "    {} = {value}", python_name(variant)).unwrap();
        }
        module.push_str(&wire_names_line(
            enum_name,
            variants.iter().map(|(variant, _)| variant.as_str()),
        ));
    }

    for (enum_name, variants) in data_enums {
        // The wire names of the variants go to `s.denu` explicitly, so the
        // enum needs no `__egui_names__`.
        let names: Vec<_> = variants
            .iter()
            .map(|(variant, _)| variant_name(variant))
            .collect();
        for (index, name) in names.iter().enumerate() {
            if names[..index].contains(name) {
                panic!("{enum_name}: two variants have the Python name `{name}`");
            }
        }
        writeln!(
            module,
            "\n\nclass {enum_name}(s._CustomEnum):\n    \"\"\"A generated protocol enum with data; a value is a ``{enum_name}.{VARIANTS_ALIAS}``.\"\"\""
        )
        .unwrap();

        for ((variant, fields), attr) in variants.iter().zip(&names) {
            // A tuple variant prints its fields without their `_0` names.
            let (options, base) = match fields {
                VariantFields::Tuple(_) => (", repr=False", "(s._TupleVariant)"),
                VariantFields::Unit | VariantFields::Named(_) => ("", ""),
            };
            writeln!(
                module,
                "\n    @final\n    @dataclass(frozen=True, slots=True{options})\n    class {attr}{base}:\n        \"\"\"Protocol variant ``{enum_name}.{attr}``.\"\"\""
            )
            .unwrap();
            let python_fields = variant_fields(fields);
            if !python_fields.is_empty() {
                module.push('\n');
            }
            for (field, typ) in &python_fields {
                writeln!(module, "        {field}: {typ}").unwrap();
            }
            if let VariantFields::Named(named) = fields {
                let wire_names = wire_names_line(
                    &format!("{enum_name}.{variant}"),
                    named.iter().map(|(field, _)| field.as_str()),
                );
                if !wire_names.is_empty() {
                    write!(module, "    {wire_names}").unwrap();
                }
            }
        }

        writeln!(
            module,
            "\n    type {VARIANTS_ALIAS} = {}",
            names.join(" | ")
        )
        .unwrap();
    }

    module
}

fn render_python<S: State>() -> (String, String, String) {
    let (states, version_hash, _) = scripts::parse_states::<S>();
    scripts::validate_states(&states);

    let mut values_list = Vec::new();
    scripts::states_into_values_list(&states, &mut values_list);

    let scripts::CustomTypes {
        enums,
        data_enums,
        structs,
    } = scripts::get_custom_types(&values_list);
    let types_list = collect_object_types(&values_list);

    // Ordering structs for Python generation, so that dependencies are defined before they are used.
    let mut struct_order = VecDeque::new();
    for (struct_name, items) in &structs {
        if !struct_order.contains(struct_name) {
            struct_order.push_front(struct_name.clone());
            order_structs(items, &mut struct_order);
        }
    }

    let enums_output = render_enums(&enums, &data_enums);

    let mut structs_output = String::from(
        "# Generated by build.rs, do not edit\n# ruff: noqa: E501 F401\n\"\"\"Custom dataclasses used by the generated state server.\"\"\"\n\nfrom __future__ import annotations\n\nfrom dataclasses import dataclass\n\nimport egui_states.structures as s\n\nfrom . import enums\n",
    );
    for struct_name in &struct_order {
        let fields = &structs[struct_name];
        writeln!(
            structs_output,
            "\n\n@dataclass\nclass {struct_name}(s._CustomStruct):\n    \"\"\"A generated protocol data structure.\"\"\""
        )
        .unwrap();
        if fields.is_empty() {
            structs_output.push_str("    pass\n");
        } else {
            for (field, typ) in fields {
                let typ = type_info_to_python_type(typ, true, PythonTypeContext::Structs);
                writeln!(structs_output, "    {}: {typ}", python_name(field)).unwrap();
            }
            structs_output.push_str(&wire_names_line(
                struct_name,
                fields.iter().map(|(field, _)| field.as_str()),
            ));
        }
    }

    let mut states_output = String::from(
        "# Generated by build.rs, do not edit\n# ruff: noqa: E501\n\"\"\"Typed Python server bindings generated from the egui client state tree.\"\"\"\n\nfrom __future__ import annotations\n\nfrom collections.abc import Callable, Sequence\nfrom os import PathLike\n\nimport numpy as np\n\nimport egui_states.structures as s\nfrom egui_states.server import ClientPolicy, StatesBase, StateServerBase\nfrom egui_states.transfer import TransferProfile\nfrom egui_states.structures import ISubStates\n\nfrom . import enums, structs\n",
    );

    let StateType::SubState(_, root_name, substates, _) = &states else {
        panic!("Root state must be a SubState");
    };

    let mut used_states = Vec::new();
    for state in substates {
        if let StateType::SubState(_, state_class, sub_states, _) = state {
            if used_states.iter().any(|used| used == state_class) {
                continue;
            }
            used_states.push(state_class.to_string());
            write_states(
                &mut states_output,
                state_class,
                sub_states,
                &types_list,
                &mut used_states,
            );
        }
    }

    let mut wire_names = wire_names_line(root_name, substates.iter().map(StateType::name));
    if !wire_names.is_empty() {
        wire_names.push('\n');
    }
    writeln!(
        states_output,
        "\n\nclass {root_name}(StatesBase):\n    \"\"\"The generated root state tree.\"\"\"\n\n{wire_names}    @staticmethod\n    def _get_obj_types() -> list[s.PyObjectType]:\n        return ["
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
        "        ]\n\n    def __init__(self, server: StateServerBase) -> None:\n        \"\"\"Initialize the generated root state handles.\n\n        Args:\n            server (StateServerBase): State server that owns this tree.\n        \"\"\"\n        super().__init__(server)\n        parent = \"root\"\n",
    );
    for state in substates {
        states_output.push_str(&state_to_line(state, &types_list, false));
    }

    write!(
        states_output,
        r#"

class StatesServer(StateServerBase[{root_name}]):
    """Ready-to-use server with the generated state tree."""

    #: Stable hash of the generated state layout.
    VERSION_HASH: int = {version_hash}

    def __init__(
        self,
        signals_workers: int = 3,
        error_handler: Callable[[Exception], None] | None = None,
        version: int | None = None,
        heartbeat_interval: float = 5.0,
        heartbeat_timeout: float | None = 20.0,
        web_path: str | PathLike[str] | None = None,
        allowed_origins: Sequence[str] = (),
        transfer_profile: TransferProfile = TransferProfile.Balanced,
        client_policy: ClientPolicy = ClientPolicy.Replace,
        connect_request_timeout: float = 5.0,
        tls: bool = False,
        tls_cert: str | PathLike[str] | None = None,
        tls_key: str | PathLike[str] | None = None,
    ) -> None:
        """Initialize and finalize the generated state server.

        Args:
            signals_workers (int, optional): Number of callback worker threads.
                Defaults to 3.
            error_handler (Callable[[Exception], None] | None, optional):
                Handler for callback and signal-worker exceptions. Defaults to
                the traceback-printing handler.
            version (int | None, optional): Application version required from
                the client.
            heartbeat_interval (float, optional): Seconds between the pings the
                server sends to the connected client. Defaults to 5.
            heartbeat_timeout (float | None, optional): Seconds the client may
                send nothing before it is disconnected. ``None`` disables the
                heartbeat. Defaults to 20.
            web_path (str | PathLike[str] | None, optional): Directory of a
                static web page, such as a built WASM client, that
                ``start_web`` serves on the server port. Defaults to no page.
            allowed_origins (Sequence[str], optional): Origin patterns of
                other web pages that may connect, such as
                ``http://localhost:8090`` or ``http://myserver.corp.example:*``;
                ``*`` alone allows every page. Defaults to pages served by the
                server itself.
            transfer_profile (TransferProfile, optional): How file transfers
                share the connection with state synchronization:
                ``Responsive`` for slow links, ``Balanced``, or ``Throughput``
                for a client on the same machine. Defaults to ``Balanced``.
            client_policy (ClientPolicy, optional): What the server does with
                a client that connects while another is connected:
                ``Replace`` lets it take over, ``Reject`` refuses it as busy.
                Defaults to ``Replace``.
            connect_request_timeout (float, optional): Seconds a callback
                registered with ``on_connect_request`` may take to decide on a
                client before it is refused. Defaults to 5.
            tls (bool, optional): Whether clients connect with TLS, which
                encrypts the connection. Without ``tls_cert``, the server
                generates a self-signed certificate that exists only in
                memory; see ``tls_fingerprint``. Defaults to ``False``.
            tls_cert (str | PathLike[str] | None, optional): PEM file with the
                certificate to present instead of a generated one. Requires
                ``tls`` and ``tls_key``. Defaults to a generated certificate.
            tls_key (str | PathLike[str] | None, optional): PEM file with the
                unencrypted private key of ``tls_cert``.

        Raises:
            ValueError: If ``heartbeat_interval`` is not positive,
                ``heartbeat_timeout`` is not greater than it, ``web_path``
                cannot be made absolute, an allowed origin is malformed,
                ``transfer_profile`` is not a ``TransferProfile``,
                ``client_policy`` is not a ``ClientPolicy``,
                ``connect_request_timeout`` is not positive, or the TLS
                settings or files are invalid.
            OSError: If ``tls_cert`` or ``tls_key`` cannot be read.
        """
        super().__init__(
            {root_name},
            signals_workers,
            error_handler,
            version,
            heartbeat_interval,
            heartbeat_timeout,
            web_path,
            allowed_origins,
            transfer_profile,
            client_policy,
            connect_request_timeout,
            tls,
            tls_cert,
            tls_key,
        )
"#
    )
    .unwrap();

    (states_output, enums_output, structs_output)
}

/// Generates typed Python server bindings as a three-file package.
///
/// The output directory is created when necessary and receives `__init__.py`,
/// `enums.py`, and `structs.py`. Files whose contents did not change are left
/// untouched so the generator does not trigger unnecessary downstream work.
/// Add the output directory's parent to Python's import path and import the
/// generated `StatesServer` from the package.
///
/// Generated files must not be edited by hand; rebuild them from the shared
/// Rust [`State`] definition instead.
///
/// State, enum variant, and struct field names that are Python keywords get a
/// trailing underscore (`from` becomes `from_`), and so does a variant called
/// `Type` of an enum with data, whose `Type` is the union of its variants. The
/// original names are kept on the wire, so the bindings stay compatible with
/// the Rust client.
///
/// # Errors
///
/// Returns an error if the output directory or files cannot be created or
/// written.
///
/// # Panics
///
/// Panics if a state type is defined inconsistently in different locations or
/// conflicts with a generated symbol such as `StatesServer`, `enums`,
/// `structs`, or `s`.
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
