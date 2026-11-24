//! Valid Rust inputs that are awkward for generated code: target-language
//! keywords, raw identifiers, names the generated modules use themselves,
//! literals without a plain spelling, and a unit struct.
use egui_states::{State, StatesCreator, Value};

#[egui_states::typed(rust_derive(Clone, Copy, Debug, PartialEq))]
#[derive(Clone, Copy, Default, egui_states::InitialValue)]
pub enum Keyword {
    #[default]
    None,
    True,
    Plain,
}

#[egui_states::typed(rust_derive(Clone, Debug, PartialEq))]
#[derive(Clone, Default, egui_states::InitialValue)]
pub struct Reserved {
    pub from: u32,
    pub lambda: bool,
}

#[egui_states::typed(rust_derive(Clone, Debug, PartialEq))]
#[derive(Clone, Default, egui_states::InitialValue)]
pub struct Marker;

/// Raw identifiers as struct fields.
#[egui_states::typed(rust_derive(Clone, Debug, PartialEq))]
#[derive(Clone, Default, egui_states::InitialValue)]
pub struct RawFields {
    pub r#type: u8,
    pub r#in: bool,
}

/// Raw identifiers as the variants of an enum with data and as their fields.
#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Default, egui_states::InitialValue)]
#[allow(non_camel_case_types)]
pub enum RawCommand {
    #[default]
    Stop,
    r#match {
        r#type: u8,
        r#if: bool,
    },
    r#try(u8),
}

/// Types and variants named like what the generated modules use themselves:
/// the Rust prelude, and the imports of the Python `enums` module.
pub mod shadowing {
    #[egui_states::typed(rust_derive(Debug, PartialOrd))]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    pub enum Option {
        #[default]
        Off,
        On,
    }

    #[egui_states::typed(rust_derive(Debug))]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    #[allow(non_camel_case_types)]
    pub enum s {
        #[default]
        First,
        Second,
    }

    #[egui_states::typed(rust_derive(Debug, PartialEq))]
    #[derive(Clone, Default, egui_states::InitialValue)]
    #[allow(non_camel_case_types)]
    pub enum String {
        #[default]
        s,
        dataclass(u8),
        r#final {
            text: ::std::string::String,
            maybe: ::std::option::Option<s>,
            choices: ::std::vec::Vec<Option>,
        },
        IntEnum(::std::collections::HashMap<u8, ::std::string::String>),
    }
}

/// Types named like Python keywords. Python cannot define them, so only the
/// Rust generator takes them.
pub mod keyword_types {
    #[egui_states::typed(rust_derive(Clone, Copy, Debug, PartialEq))]
    #[derive(Clone, Copy, Default, egui_states::InitialValue)]
    pub enum None {
        #[default]
        Nothing,
        Something,
    }

    #[egui_states::typed(rust_derive(Debug, PartialEq))]
    #[derive(Clone, Default, egui_states::InitialValue)]
    pub enum True {
        #[default]
        Yes,
        Maybe(::std::option::Option<None>),
    }

    #[egui_states::typed(rust_derive(Clone, Debug, PartialEq))]
    #[derive(Clone, Default, egui_states::InitialValue)]
    #[allow(non_camel_case_types)]
    pub struct lambda {
        pub value: u8,
        pub choice: None,
        pub maybe: ::std::option::Option<u8>,
    }
}

pub const ESCAPED: &str = "quote \" backslash \\ tab \t newline \n control \u{1} delete \u{7f}";

/// Python keywords and raw identifiers as enum variants, struct fields, and
/// state names, next to types that share a name with the generated code.
pub struct KeywordNames;

impl State for KeywordNames {
    const NAME: &'static str = "KeywordNames";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<Keyword> = c.value("keyword", Keyword::True);
        let _: Value<Reserved> = c.value(
            "reserved",
            Reserved {
                from: 3,
                lambda: true,
            },
        );
        let _: Value<u8> = c.value("global", 1);
        let _: Value<RawFields> = c.value(
            "r#type",
            RawFields {
                r#type: 5,
                r#in: true,
            },
        );
        let _: Value<RawCommand> = c.value(
            "raw_command",
            RawCommand::r#match {
                r#type: 1,
                r#if: true,
            },
        );
        let _: Value<shadowing::Option> = c.value("option", shadowing::Option::On);
        let _: Value<shadowing::s> = c.value("short", shadowing::s::Second);
        let _: Value<shadowing::String> = c.value(
            "shadowed",
            shadowing::String::r#final {
                text: "text".into(),
                maybe: Some(shadowing::s::Second),
                choices: vec![shadowing::Option::On],
            },
        );
        let _: Value<shadowing::String> = c.value(
            "shadowed_map",
            shadowing::String::IntEnum([(1, "one".into())].into()),
        );
        Self
    }
}

/// Initial values whose Rust spelling is not valid in every target language.
pub struct SpecialLiterals;

impl State for SpecialLiterals {
    const NAME: &'static str = "SpecialLiterals";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<String> = c.value("escaped", ESCAPED.to_string());
        let _: Value<f64> = c.value("not_a_number", f64::NAN);
        let _: Value<f64> = c.value("positive_infinity", f64::INFINITY);
        let _: Value<f64> = c.value("negative_infinity", f64::NEG_INFINITY);
        let _: Value<f32> = c.value("single_not_a_number", f32::NAN);
        Self
    }
}

/// A value of a field-less struct.
pub struct UnitStruct;

impl State for UnitStruct {
    const NAME: &'static str = "UnitStruct";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<Marker> = c.value("marker", Marker);
        Self
    }
}

/// A state group and types named like Python keywords, for the Rust
/// generator alone.
pub struct KeywordTypes;

impl State for KeywordTypes {
    const NAME: &'static str = "None";

    fn new(c: &mut impl StatesCreator) -> Self {
        use keyword_types as types;
        let _: Value<types::None> = c.value("nothing", types::None::Something);
        let _: Value<types::True> =
            c.value("truth", types::True::Maybe(Some(types::None::Something)));
        let _: Value<types::lambda> = c.value(
            "function",
            types::lambda {
                value: 2,
                choice: types::None::Something,
                maybe: Option::None,
            },
        );
        Self
    }
}

/// All awkward inputs, for the Rust generator.
pub struct Awkward;

impl State for Awkward {
    const NAME: &'static str = "Awkward";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: KeywordNames = c.substate("keywords");
        let _: SpecialLiterals = c.substate("literals");
        let _: UnitStruct = c.substate("unit");
        let _: KeywordTypes = c.substate("keyword_types");
        Self
    }
}
