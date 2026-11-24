use std::collections::HashMap;
use std::fmt::Debug;

use egui_states::{ObjectType, RustDerive, Typed};

#[egui_states::typed]
#[derive(Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct MacroStruct {
    count: u32,
    #[serde(rename = "displayLabel")]
    label: String,
}

#[egui_states::typed]
#[derive(Debug, PartialEq)]
enum MacroEnum {
    Negative = -2,
    Next,
    Positive = 4,
}

#[egui_states::typed]
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "client", derive(egui_states::Atomic))]
#[repr(i32)]
enum BoundaryEnum {
    Min = -2147483648,
    AfterMin,
    Negative = -2,
    AfterNegative,
    Zero,
    Positive = 4,
    Max = 2147483647,
}

const BOUNDARY_VALUES: [BoundaryEnum; 7] = [
    BoundaryEnum::Min,
    BoundaryEnum::AfterMin,
    BoundaryEnum::Negative,
    BoundaryEnum::AfterNegative,
    BoundaryEnum::Zero,
    BoundaryEnum::Positive,
    BoundaryEnum::Max,
];

#[egui_states::typed]
#[derive(Debug, PartialEq, egui_states::serde::Serialize)]
struct SerializeOnly {
    value: bool,
}

#[egui_states::typed]
#[derive(Debug, PartialEq, egui_states::serde::Deserialize)]
struct DeserializeOnly {
    value: i16,
}

#[egui_states::typed]
#[derive(Debug, PartialEq, egui_states::serde::Serialize, egui_states::serde::Deserialize)]
#[serde(crate = "egui_states::serde")]
struct ExistingSerdeDerives {
    value: f32,
}

#[egui_states::typed]
#[derive(Debug, PartialEq)]
#[serde(transparent)]
struct TransparentWrapper {
    value: u32,
}

#[egui_states::typed(rust_derive(Debug, server_macros::CustomDerive))]
#[derive(Clone, Debug, PartialEq)]
struct DerivedInner {
    value: u32,
}

#[egui_states::typed(rust_derive(Eq, Hash))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DerivedKey {
    Value,
}

#[egui_states::typed(rust_derive(PartialEq))]
#[derive(Clone, Debug, PartialEq)]
struct DerivedOuter {
    inner: Option<Vec<DerivedInner>>,
    values: HashMap<DerivedKey, bool>,
}

fn assert_traits<T>()
where
    T: Typed + egui_states::serde::Serialize + for<'de> egui_states::serde::Deserialize<'de>,
{
}

fn assert_round_trip<T>(value: T)
where
    T: Debug
        + PartialEq
        + egui_states::serde::Serialize
        + for<'de> egui_states::serde::Deserialize<'de>,
{
    let bytes = postcard::to_stdvec(&value).unwrap();
    let deserialized = postcard::from_bytes::<T>(&bytes).unwrap();
    assert_eq!(deserialized, value);
}

#[test]
fn typed_attribute_preserves_struct_type_information() {
    assert_traits::<MacroStruct>();
    assert_eq!(
        MacroStruct::get_type(),
        ObjectType::Struct(
            "MacroStruct".to_string(),
            vec![
                ("count".to_string(), ObjectType::U32),
                ("label".to_string(), ObjectType::String),
            ],
        )
    );
    assert_round_trip(MacroStruct {
        count: 42,
        label: "example".to_string(),
    });
}

#[test]
fn typed_attribute_preserves_enum_discriminants() {
    assert_traits::<MacroEnum>();
    assert_eq!(
        MacroEnum::get_type(),
        ObjectType::Enum(
            "MacroEnum".to_string(),
            vec![
                ("Negative".to_string(), -2),
                ("Next".to_string(), -1),
                ("Positive".to_string(), 4),
            ],
        )
    );
    assert_round_trip(MacroEnum::Negative);
    assert_round_trip(MacroEnum::Next);
    assert_round_trip(MacroEnum::Positive);
}

#[test]
fn typed_attribute_preserves_i32_boundaries_and_wire_values() {
    assert_traits::<BoundaryEnum>();
    assert_eq!(
        BoundaryEnum::get_type(),
        ObjectType::Enum(
            "BoundaryEnum".to_string(),
            vec![
                ("Min".to_string(), i32::MIN),
                ("AfterMin".to_string(), i32::MIN + 1),
                ("Negative".to_string(), -2),
                ("AfterNegative".to_string(), -1),
                ("Zero".to_string(), 0),
                ("Positive".to_string(), 4),
                ("Max".to_string(), i32::MAX),
            ],
        )
    );
    for (index, value) in BOUNDARY_VALUES.into_iter().enumerate() {
        // Serde/Postcard encodes the variant's ordinal, not its discriminant.
        assert_eq!(postcard::to_stdvec(&value).unwrap(), [index as u8]);
        assert_round_trip(value);
    }
}

#[cfg(feature = "client")]
#[test]
fn atomic_derive_preserves_i32_boundaries() {
    use egui_states::Atomic;

    for value in BOUNDARY_VALUES {
        assert_eq!(BoundaryEnum::from_atomic(value.to_atomic()), value);
    }
}

#[cfg(feature = "client")]
#[test]
#[should_panic(expected = "Invalid enum value for BoundaryEnum: 7")]
fn atomic_derive_rejects_an_unknown_discriminant() {
    use egui_states::Atomic;

    BoundaryEnum::from_atomic(7);
}

#[test]
fn typed_attribute_adds_only_missing_serde_derives() {
    assert_traits::<SerializeOnly>();
    assert_traits::<DeserializeOnly>();
    assert_traits::<ExistingSerdeDerives>();

    assert_round_trip(SerializeOnly { value: true });
    assert_round_trip(DeserializeOnly { value: -7 });
    assert_round_trip(ExistingSerdeDerives { value: 1.25 });
}

#[test]
fn typed_attribute_accepts_wire_neutral_transparent_wrapper() {
    assert_traits::<TransparentWrapper>();
    assert_eq!(
        TransparentWrapper::get_type(),
        ObjectType::Struct(
            "TransparentWrapper".to_string(),
            vec![("value".to_string(), ObjectType::U32)],
        )
    );
    for value in [0, 127, 128, u32::MAX] {
        let wrapper = TransparentWrapper { value };
        // Peers encode the described single-field struct as the bare field.
        assert_eq!(
            postcard::to_stdvec(&wrapper).unwrap(),
            postcard::to_stdvec(&value).unwrap()
        );
        assert_round_trip(wrapper);
    }
}

#[test]
fn typed_attribute_reports_recursive_rust_derives() {
    let derives = DerivedOuter::rust_derives();
    assert_eq!(derives.len(), 3);

    assert_eq!(
        derives[0],
        RustDerive::Struct("DerivedOuter", &["PartialEq"])
    );
    match derives[1] {
        RustDerive::Struct("DerivedInner", derives) => {
            assert_eq!(derives[0], "Debug");
            assert_eq!(derives[1].replace(' ', ""), "server_macros::CustomDerive");
        }
        _ => panic!("expected DerivedInner struct metadata"),
    }
    assert_eq!(derives[2], RustDerive::Enum("DerivedKey", &["Eq", "Hash"]));
}

#[test]
fn typed_attribute_reports_empty_rust_derives() {
    assert_eq!(
        MacroStruct::rust_derives(),
        [RustDerive::Struct("MacroStruct", &[])]
    );
    assert_eq!(
        MacroEnum::rust_derives(),
        [RustDerive::Enum("MacroEnum", &[])]
    );
}

#[egui_states::typed(rust_derive(Hash))]
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "client", derive(egui_states::InitialValue))]
struct CommandTarget {
    id: u16,
}

#[egui_states::typed(rust_derive(Eq))]
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "client", derive(egui_states::InitialValue))]
enum CommandMode {
    Fast,
    Slow,
}

#[egui_states::typed(rust_derive(Debug))]
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "client", derive(egui_states::InitialValue))]
enum MacroCommand {
    Stop,
    MoveTo {
        x: f32,
        inner: Option<CommandTarget>,
    },
    Pair(u8, String),
    Wrap(CommandMode),
}

#[test]
fn typed_attribute_describes_enums_with_data() {
    use egui_states::VariantFields;

    assert_traits::<MacroCommand>();
    let ObjectType::DataEnum(name, variants) = MacroCommand::get_type() else {
        panic!("expected an enum with data");
    };
    assert_eq!(name, "MacroCommand");
    let names: Vec<_> = variants.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Stop", "MoveTo", "Pair", "Wrap"]);
    assert_eq!(variants[0].1, VariantFields::Unit);
    assert!(
        matches!(&variants[1].1, VariantFields::Named(fields) if fields[0] == ("x".to_string(), ObjectType::F32) && fields[1].0 == "inner")
    );
    assert_eq!(
        variants[2].1,
        VariantFields::Tuple(vec![ObjectType::U8, ObjectType::String])
    );
    assert_eq!(
        variants[3].1,
        VariantFields::Tuple(vec![CommandMode::get_type()])
    );

    // Postcard sends the variant's position, then its fields in order.
    assert_eq!(postcard::to_stdvec(&MacroCommand::Stop).unwrap(), [0]);
    assert_eq!(
        postcard::to_stdvec(&MacroCommand::Pair(7, "ab".into())).unwrap(),
        [2, 7, 2, b'a', b'b']
    );
    assert_round_trip(MacroCommand::MoveTo {
        x: 1.5,
        inner: Some(CommandTarget { id: 4 }),
    });
    assert_round_trip(MacroCommand::Wrap(CommandMode::Slow));

    // Field types report their own derives after the enum's.
    let derives = MacroCommand::rust_derives();
    assert_eq!(derives[0], RustDerive::Enum("MacroCommand", &["Debug"]));
    assert_eq!(
        derives[1..],
        [
            RustDerive::Struct("CommandTarget", &["Hash"]),
            RustDerive::Enum("CommandMode", &["Eq"]),
        ]
    );
}

#[cfg(feature = "client")]
#[test]
fn initial_values_of_enums_with_data_carry_their_fields() {
    use egui_states::{InitValue, InitialValue};
    assert_eq!(
        MacroCommand::Stop.init_value(),
        InitValue::DataEnum("Stop", vec![])
    );
    assert_eq!(
        MacroCommand::Pair(3, "x".into()).init_value(),
        InitValue::DataEnum(
            "Pair",
            vec![InitValue::U8(3), InitValue::String("x".into())]
        )
    );
    assert_eq!(
        MacroCommand::MoveTo {
            x: 0.5,
            inner: None
        }
        .init_value(),
        InitValue::DataEnum("MoveTo", vec![InitValue::F32(0.5), InitValue::Option(None)])
    );
}

mod mouse {
    #[egui_states::typed]
    #[derive(Debug, PartialEq)]
    pub enum Event {
        Down(u8),
        Up,
    }

    #[egui_states::typed]
    #[derive(Debug, PartialEq)]
    pub struct Position {
        pub x: i32,
    }
}

/// Shares its name with a type of another module, which it contains.
#[egui_states::typed]
#[derive(Debug, PartialEq)]
enum Event {
    Mouse(mouse::Event),
    Key { code: u32 },
}

#[egui_states::typed]
#[derive(Debug, PartialEq)]
struct Position {
    mouse: mouse::Position,
    history: Vec<mouse::Position>,
}

/// A constant in the value namespace, named like the enum below.
#[allow(non_upper_case_globals)]
const Data: usize = 4;

#[egui_states::typed]
#[derive(Debug, PartialEq)]
enum Data {
    Bytes([u8; Data]),
    Empty,
}

/// A type that only shares its name with one of its fields does not contain
/// itself, for the macro and for the guard of `get_type`.
#[test]
fn a_type_may_contain_another_type_of_the_same_name() {
    use egui_states::VariantFields;

    assert_eq!(
        Event::get_type(),
        ObjectType::DataEnum(
            "Event".to_string(),
            vec![
                (
                    "Mouse".to_string(),
                    VariantFields::Tuple(vec![mouse::Event::get_type()])
                ),
                (
                    "Key".to_string(),
                    VariantFields::Named(vec![("code".to_string(), ObjectType::U32)])
                ),
            ],
        )
    );
    assert_eq!(
        Event::rust_derives(),
        [
            RustDerive::Enum("Event", &[]),
            RustDerive::Enum("Event", &[])
        ]
    );
    assert_round_trip(Event::Mouse(mouse::Event::Down(2)));

    let position = ObjectType::Struct(
        "Position".to_string(),
        vec![("x".to_string(), ObjectType::I32)],
    );
    assert_eq!(
        Position::get_type(),
        ObjectType::Struct(
            "Position".to_string(),
            vec![
                ("mouse".to_string(), position.clone()),
                ("history".to_string(), ObjectType::Vec(Box::new(position))),
            ],
        )
    );
    assert_eq!(Position::rust_derives().len(), 3);

    assert_eq!(
        Data::get_type(),
        ObjectType::DataEnum(
            "Data".to_string(),
            vec![
                (
                    "Bytes".to_string(),
                    VariantFields::Tuple(vec![ObjectType::List(4, Box::new(ObjectType::U8))])
                ),
                ("Empty".to_string(), VariantFields::Unit),
            ],
        )
    );
}

/// Forwards discriminants the way a `macro_rules!` generated enum does.
macro_rules! forwarded_enum {
    ($name:ident, $first:expr, $second:expr) => {
        #[egui_states::typed]
        #[derive(Debug, PartialEq)]
        enum $name {
            First = $first,
            Second = $second,
            Third,
        }
    };
}

forwarded_enum!(ForwardedEnum, 3, -7);

#[test]
fn typed_attribute_reads_discriminants_forwarded_by_a_macro() {
    assert_eq!(
        ForwardedEnum::get_type(),
        ObjectType::Enum(
            "ForwardedEnum".to_string(),
            vec![
                ("First".to_string(), 3),
                ("Second".to_string(), -7),
                ("Third".to_string(), -6),
            ],
        )
    );
    assert_eq!(ForwardedEnum::Third as i32, -6);
}

/// Two types that contain each other; neither names itself.
#[egui_states::typed]
struct Ping {
    pongs: Vec<Pong>,
}

#[egui_states::typed]
enum Pong {
    Last,
    Back(Option<Ping>),
}

/// Hides the type from the macro, which sees only the alias.
type Itself = Looped;

#[egui_states::typed]
struct Looped {
    again: Vec<Itself>,
}

/// The message of the panic of `describe`.
fn description_panic<R>(describe: fn() -> R) -> String {
    let panic = std::panic::catch_unwind(describe)
        .err()
        .expect("the type was described");
    *panic.downcast::<String>().expect("a formatted message")
}

/// A type the macro cannot see containing itself is stopped when it is
/// described, with its name instead of a stack overflow.
#[test]
fn describing_an_indirectly_recursive_type_panics() {
    const UNSUPPORTED: &str = "contains itself; recursive types are not supported by `#[typed]`";

    assert_eq!(
        description_panic(Ping::get_type),
        format!("Type Ping {UNSUPPORTED}")
    );
    assert_eq!(
        description_panic(Pong::get_type),
        format!("Type Pong {UNSUPPORTED}")
    );
    assert_eq!(
        description_panic(Ping::rust_derives),
        format!("Type Ping {UNSUPPORTED}")
    );
    assert_eq!(
        description_panic(Looped::get_type),
        format!("Type Looped {UNSUPPORTED}")
    );
    assert_eq!(
        description_panic(Looped::rust_derives),
        format!("Type Looped {UNSUPPORTED}")
    );

    // The panics left nothing behind: the same type is rejected again, and
    // others are described as before.
    assert_eq!(
        description_panic(Ping::get_type),
        format!("Type Ping {UNSUPPORTED}")
    );
    assert_eq!(DerivedOuter::rust_derives().len(), 3);
    assert!(matches!(MacroCommand::get_type(), ObjectType::DataEnum(..)));
}
