use std::collections::VecDeque;
use std::fmt::Write;
use std::path::Path;

use crate::InitValue;
use crate::State;
use crate::build_scripts::scripts;
use crate::build_scripts::states_creator_build::StateType;
use crate::data_transport::DataType;
use crate::typed::ObjectType;

fn data_type_to_rust_type(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::U8 => "u8",
        DataType::U16 => "u16",
        DataType::U32 => "u32",
        DataType::U64 => "u64",
        DataType::I8 => "i8",
        DataType::I16 => "i16",
        DataType::I32 => "i32",
        DataType::I64 => "i64",
        DataType::F32 => "f32",
        DataType::F64 => "f64",
    }
}

#[derive(Clone, Copy)]
enum RustTypeContext {
    States,
    Structs,
}

fn type_to_rust_type(object_type: &ObjectType, context: RustTypeContext) -> String {
    match object_type {
        ObjectType::U8 => "u8".to_string(),
        ObjectType::U16 => "u16".to_string(),
        ObjectType::U32 => "u32".to_string(),
        ObjectType::U64 => "u64".to_string(),
        ObjectType::I8 => "i8".to_string(),
        ObjectType::I16 => "i16".to_string(),
        ObjectType::I32 => "i32".to_string(),
        ObjectType::I64 => "i64".to_string(),
        ObjectType::F32 => "f32".to_string(),
        ObjectType::F64 => "f64".to_string(),
        ObjectType::Bool => "bool".to_string(),
        ObjectType::String => "String".to_string(),
        ObjectType::Empty => "()".to_string(),
        ObjectType::Enum(name, _) => match context {
            RustTypeContext::States => format!("enums::{name}"),
            RustTypeContext::Structs => format!("super::enums::{name}"),
        },
        ObjectType::Struct(name, _) => match context {
            RustTypeContext::States => format!("structs::{name}"),
            RustTypeContext::Structs => name.clone(),
        },
        ObjectType::Tuple(elements) => match elements.len() {
            0 => "()".to_string(),
            1 => format!("({},)", type_to_rust_type(&elements[0], context)),
            _ => {
                let elements = elements
                    .iter()
                    .map(|element| type_to_rust_type(element, context))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({elements})")
            }
        },
        ObjectType::List(size, element) => {
            format!("[{}; {size}]", type_to_rust_type(element, context))
        }
        ObjectType::Vec(element) => format!("Vec<{}>", type_to_rust_type(element, context)),
        ObjectType::Map(key, value) => format!(
            "std::collections::HashMap<{}, {}>",
            type_to_rust_type(key, context),
            type_to_rust_type(value, context)
        ),
        ObjectType::Option(element) => {
            format!("Option<{}>", type_to_rust_type(element, context))
        }
    }
}

fn float_value(value: f64, suffix: &str) -> String {
    if value.is_nan() {
        format!("{suffix}::NAN")
    } else if value == f64::INFINITY {
        format!("{suffix}::INFINITY")
    } else if value == f64::NEG_INFINITY {
        format!("{suffix}::NEG_INFINITY")
    } else {
        format!("{value:?}{suffix}")
    }
}

fn init_to_rust_value(init: &InitValue, object_type: &ObjectType) -> String {
    match (init, object_type) {
        (InitValue::U8(value), ObjectType::U8) => format!("{value}u8"),
        (InitValue::U16(value), ObjectType::U16) => format!("{value}u16"),
        (InitValue::U32(value), ObjectType::U32) => format!("{value}u32"),
        (InitValue::U64(value), ObjectType::U64) => format!("{value}u64"),
        (InitValue::I8(value), ObjectType::I8) => format!("{value}i8"),
        (InitValue::I16(value), ObjectType::I16) => format!("{value}i16"),
        (InitValue::I32(value), ObjectType::I32) => format!("{value}i32"),
        (InitValue::I64(value), ObjectType::I64) => format!("{value}i64"),
        (InitValue::F32(value), ObjectType::F32) => float_value(*value as f64, "f32"),
        (InitValue::F64(value), ObjectType::F64) => float_value(*value, "f64"),
        (InitValue::Bool(value), ObjectType::Bool) => value.to_string(),
        (InitValue::String(value), ObjectType::String) => format!("String::from({value:?})"),
        (InitValue::Enum(variant), ObjectType::Enum(name, _)) => {
            format!("enums::{name}::{variant}")
        }
        (InitValue::Option(None), ObjectType::Option(element)) => format!(
            "None::<{}>",
            type_to_rust_type(element, RustTypeContext::States)
        ),
        (InitValue::Option(Some(value)), ObjectType::Option(element)) => {
            format!("Some({})", init_to_rust_value(value, element))
        }
        (InitValue::Tuple(values), ObjectType::Tuple(types)) => match values.len() {
            0 => "()".to_string(),
            1 => format!("({},)", init_to_rust_value(&values[0], &types[0])),
            _ => {
                let values = values
                    .iter()
                    .zip(types.iter())
                    .map(|(value, typ)| init_to_rust_value(value, typ))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({values})")
            }
        },
        (InitValue::List(values), ObjectType::List(_, element)) => {
            let values = values
                .iter()
                .map(|value| init_to_rust_value(value, element))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        }
        (InitValue::Vec(values), ObjectType::Vec(element)) => {
            if values.is_empty() {
                format!(
                    "Vec::<{}>::new()",
                    type_to_rust_type(element, RustTypeContext::States)
                )
            } else {
                let values = values
                    .iter()
                    .map(|value| init_to_rust_value(value, element))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("vec![{values}]")
            }
        }
        (InitValue::Map(values), ObjectType::Map(key_type, value_type)) => {
            let mut values = values
                .iter()
                .map(|(key, value)| {
                    format!(
                        "({}, {})",
                        init_to_rust_value(key, key_type),
                        init_to_rust_value(value, value_type)
                    )
                })
                .collect::<Vec<_>>();
            values.sort_unstable();
            let values = values.join(", ");
            format!(
                "std::collections::HashMap::<{}, {}>::from([{values}])",
                type_to_rust_type(key_type, RustTypeContext::States),
                type_to_rust_type(value_type, RustTypeContext::States)
            )
        }
        (InitValue::Struct(name, values), ObjectType::Struct(_, fields)) => {
            let values = values
                .iter()
                .zip(fields.iter())
                .map(|((field, value), (_, typ))| {
                    format!("{field}: {}", init_to_rust_value(value, typ))
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("structs::{name} {{ {values} }}")
        }
        _ => panic!("Mismatched InitValue and ObjectType"),
    }
}

fn state_field_type(state: &StateType) -> String {
    match state {
        StateType::Value(_, typ, _, _) => format!(
            "s::Value<{}>",
            type_to_rust_type(typ, RustTypeContext::States)
        ),
        StateType::ValueTake(_, typ) => format!(
            "s::ValueTake<{}>",
            type_to_rust_type(typ, RustTypeContext::States)
        ),
        StateType::Static(_, typ, _) => format!(
            "s::Static<{}>",
            type_to_rust_type(typ, RustTypeContext::States)
        ),
        StateType::Signal(_, typ, _) => format!(
            "s::Signal<{}>",
            type_to_rust_type(typ, RustTypeContext::States)
        ),
        StateType::ValueVec(_, typ) => format!(
            "s::VecState<{}>",
            type_to_rust_type(typ, RustTypeContext::States)
        ),
        StateType::ValueMap(_, key, value) => format!(
            "s::MapState<{}, {}>",
            type_to_rust_type(key, RustTypeContext::States),
            type_to_rust_type(value, RustTypeContext::States)
        ),
        StateType::Data(_, typ) => format!("s::Data<{}>", data_type_to_rust_type(typ)),
        StateType::DataTake(_, typ) => format!("s::DataTake<{}>", data_type_to_rust_type(typ)),
        StateType::DataMulti(_, typ) => {
            format!("s::DataMulti<{}>", data_type_to_rust_type(typ))
        }
        StateType::DataMultiTake(_, typ) => {
            format!("s::DataMultiTake<{}>", data_type_to_rust_type(typ))
        }
        StateType::Image(_) => "s::Image".to_string(),
        StateType::SubState(_, state_class, _) => state_class.to_string(),
    }
}

fn state_initializer(state: &StateType) -> String {
    match state {
        StateType::Value(name, typ, init, queue) => format!(
            "s::Value::new(server, format!(\"{{parent}}.{}\"), {}, {})?",
            name,
            init_to_rust_value(init, typ),
            queue
        ),
        StateType::ValueTake(name, typ) => format!(
            "s::ValueTake::<{}>::new(server, format!(\"{{parent}}.{}\"))?",
            type_to_rust_type(typ, RustTypeContext::States),
            name
        ),
        StateType::Static(name, typ, init) => format!(
            "s::Static::new(server, format!(\"{{parent}}.{}\"), {})?",
            name,
            init_to_rust_value(init, typ)
        ),
        StateType::Signal(name, typ, queue) => format!(
            "s::Signal::<{}>::new(server, format!(\"{{parent}}.{}\"), {})?",
            type_to_rust_type(typ, RustTypeContext::States),
            name,
            queue
        ),
        StateType::ValueVec(name, typ) => format!(
            "s::VecState::<{}>::new(server, format!(\"{{parent}}.{}\"))?",
            type_to_rust_type(typ, RustTypeContext::States),
            name
        ),
        StateType::ValueMap(name, key, value) => format!(
            "s::MapState::<{}, {}>::new(server, format!(\"{{parent}}.{}\"))?",
            type_to_rust_type(key, RustTypeContext::States),
            type_to_rust_type(value, RustTypeContext::States),
            name
        ),
        StateType::Data(name, typ) => format!(
            "s::Data::<{}>::new(server, format!(\"{{parent}}.{}\"))?",
            data_type_to_rust_type(typ),
            name
        ),
        StateType::DataTake(name, typ) => format!(
            "s::DataTake::<{}>::new(server, format!(\"{{parent}}.{}\"))?",
            data_type_to_rust_type(typ),
            name
        ),
        StateType::DataMulti(name, typ) => format!(
            "s::DataMulti::<{}>::new(server, format!(\"{{parent}}.{}\"))?",
            data_type_to_rust_type(typ),
            name
        ),
        StateType::DataMultiTake(name, typ) => format!(
            "s::DataMultiTake::<{}>::new(server, format!(\"{{parent}}.{}\"))?",
            data_type_to_rust_type(typ),
            name
        ),
        StateType::Image(name) => {
            format!("s::Image::new(server, format!(\"{{parent}}.{}\"))?", name)
        }
        StateType::SubState(name, state_class, _) => format!(
            "{state_class}::new(server, &format!(\"{{parent}}.{}\"))?",
            name
        ),
    }
}

fn write_state(
    output: &mut String,
    state_class: &str,
    states: &[StateType],
    used_states: &mut Vec<String>,
) {
    for state in states {
        if let StateType::SubState(_, state_class, sub_states) = state {
            if used_states.iter().any(|used| used == state_class) {
                continue;
            }
            used_states.push(state_class.to_string());
            write_state(output, state_class, sub_states, used_states);
        }
    }

    writeln!(output, "\n#[derive(Clone)]\npub struct {state_class} {{").unwrap();
    for state in states {
        writeln!(
            output,
            "    pub {}: {},",
            state.name(),
            state_field_type(state)
        )
        .unwrap();
    }
    output.push_str("}\n");

    writeln!(
        output,
        "\nimpl {state_class} {{\n    pub fn new(server: &s::StateServer, parent: &str) -> s::Result<Self> {{\n        Ok(Self {{"
    )
    .unwrap();
    for state in states {
        let field = state.name();
        writeln!(output, "            {field}: {},", state_initializer(state)).unwrap();
    }
    output.push_str("        })\n    }\n}\n");
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

fn render_rust<S: State>() -> Result<(String, String, String), String> {
    let (states, version_hash) = scripts::parse_states::<S>();
    scripts::validate_states(&states);

    let mut values_list = Vec::new();
    scripts::states_into_values_list(&states, &mut values_list);
    let (enums, structs) = scripts::get_all_enums_struct(&values_list);

    let mut struct_order = VecDeque::new();
    for (struct_name, items) in &structs {
        if !struct_order.contains(struct_name) {
            struct_order.push_front(struct_name.clone());
            order_structs(items, &mut struct_order);
        }
    }

    let mut enums_output = String::from("// Generated by build.rs, do not edit\n");
    for (enum_name, variants) in &enums {
        writeln!(
            enums_output,
            "\n#[egui_states::typed]\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum {enum_name} {{"
        )
        .unwrap();
        for (variant, value) in variants {
            writeln!(enums_output, "    {variant} = {value},").unwrap();
        }
        enums_output.push_str("}\n");
    }

    let mut structs_output = String::from("// Generated by build.rs, do not edit\n");
    for struct_name in &struct_order {
        let fields = &structs[struct_name];
        writeln!(
            structs_output,
            "\n#[egui_states::typed]\n#[derive(Clone, Debug, PartialEq)]\npub struct {struct_name} {{"
        )
        .unwrap();
        for (field, typ) in fields {
            writeln!(
                structs_output,
                "    pub {field}: {},",
                type_to_rust_type(typ, RustTypeContext::Structs)
            )
            .unwrap();
        }
        structs_output.push_str("}\n");
    }

    let mut states_output = String::from(
        "// Generated by build.rs, do not edit\npub mod enums;\npub mod structs;\n\nuse egui_states::server as s;\n",
    );

    let StateType::SubState(_, root_name, substates) = &states else {
        return Err("Root state must be a SubState".to_string());
    };

    let mut used_states = Vec::new();
    for state in substates {
        if let StateType::SubState(_, state_class, sub_states) = state {
            if used_states.iter().any(|used| used == state_class) {
                continue;
            }
            used_states.push(state_class.to_string());
            write_state(
                &mut states_output,
                state_class,
                sub_states,
                &mut used_states,
            );
        }
    }

    writeln!(
        states_output,
        "\n#[derive(Clone)]\npub struct {root_name} {{"
    )
    .unwrap();
    for state in substates {
        writeln!(
            states_output,
            "    pub {}: {},",
            state.name(),
            state_field_type(state)
        )
        .unwrap();
    }
    states_output.push_str("}\n");

    writeln!(
        states_output,
        "\nimpl {root_name} {{\n    pub fn new(server: &s::StateServer) -> s::Result<Self> {{\n        let parent = \"root\";\n        Ok(Self {{"
    )
    .unwrap();
    for state in substates {
        let field = state.name();
        writeln!(
            states_output,
            "            {field}: {},",
            state_initializer(state)
        )
        .unwrap();
    }
    states_output.push_str("        })\n    }\n}\n\n");

    writeln!(
        states_output,
        "pub struct StatesServer {{\n    server: s::StateServer,\n    pub states: {root_name},\n    pub logging: s::LoggingSignal,\n}}\n"
    )
    .unwrap();
    writeln!(
        states_output,
        "impl StatesServer {{\n    pub const VERSION_HASH: u64 = {version_hash};\n\n    pub fn new(port: u16) -> s::Result<Self> {{\n        Self::with_options(s::ServerOptions::new(port))\n    }}\n\n    pub fn with_options(options: s::ServerOptions) -> s::Result<Self> {{\n        let server = s::StateServer::with_options(options)?;\n        let states = {root_name}::new(&server)?;\n        server.finalize()?;\n        let logging = s::LoggingSignal::new(&server);\n        Ok(Self {{ server, states, logging }})\n    }}\n\n    pub fn state_server(&self) -> &s::StateServer {{\n        &self.server\n    }}\n\n    pub fn start(&self) -> s::Result<()> {{\n        self.server.start()\n    }}\n\n    pub fn stop(&self) {{\n        self.server.stop();\n    }}\n\n    pub fn update(&self, duration: Option<f32>) -> s::Result<()> {{\n        self.server.update(duration)\n    }}\n\n    pub fn disconnect_client(&self) {{\n        self.server.disconnect_client();\n    }}\n\n    pub fn is_running(&self) -> bool {{\n        self.server.is_running()\n    }}\n\n    pub fn is_connected(&self) -> bool {{\n        self.server.is_connected()\n    }}\n\n    pub fn on_connect(&self, callback: impl Fn(String) + Send + Sync + 'static) -> s::CallbackHandle {{\n        self.server.on_connect(callback)\n    }}\n\n    pub fn on_disconnect(&self, callback: impl Fn() + Send + Sync + 'static) -> s::CallbackHandle {{\n        self.server.on_disconnect(callback)\n    }}\n\n    pub fn on_client_message(&self, callback: impl Fn(String) + Send + Sync + 'static) -> s::CallbackHandle {{\n        self.server.on_client_message(callback)\n    }}\n}}"
    )
    .unwrap();

    Ok((states_output, enums_output, structs_output))
}

/// Generates typed Rust server bindings as a three-file module.
pub fn generate_rust<S: State>(directory: impl AsRef<Path>) -> Result<(), String> {
    let (states, enums, structs) = render_rust::<S>()?;
    scripts::write_generated_files(
        directory.as_ref(),
        [
            ("mod.rs", states.as_str()),
            ("enums.rs", enums.as_str()),
            ("structs.rs", structs.as_str()),
        ],
    )
}
