//! Valid Rust inputs that are awkward for generated code: target-language
//! keywords, literals without a plain spelling, and a unit struct.
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

pub const ESCAPED: &str = "quote \" backslash \\ tab \t newline \n control \u{1} delete \u{7f}";

/// Python keywords as enum variants, struct fields, and a state name.
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

/// All awkward inputs, for the Rust generator.
pub struct Awkward;

impl State for Awkward {
    const NAME: &'static str = "Awkward";

    fn new(c: &mut impl StatesCreator) -> Self {
        let _: KeywordNames = c.substate("keywords");
        let _: SpecialLiterals = c.substate("literals");
        let _: UnitStruct = c.substate("unit");
        Self
    }
}
