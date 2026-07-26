use std::fmt::Debug;

use egui_states::{ObjectType, Typed};

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
fn typed_attribute_adds_only_missing_serde_derives() {
    assert_traits::<SerializeOnly>();
    assert_traits::<DeserializeOnly>();
    assert_traits::<ExistingSerdeDerives>();

    assert_round_trip(SerializeOnly { value: true });
    assert_round_trip(DeserializeOnly { value: -7 });
    assert_round_trip(ExistingSerdeDerives { value: 1.25 });
}
