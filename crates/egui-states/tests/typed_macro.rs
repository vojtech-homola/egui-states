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
#[cfg_attr(
    feature = "client",
    derive(egui_states::Atomic, egui_states::AtomicStatic)
)]
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
fn assert_boundary_storage<L: egui_states::AtomicLockStatic<BoundaryEnum>>() {
    let lock = L::new(BoundaryEnum::Max);
    assert_eq!(lock.load(), BoundaryEnum::Max);
    for value in BOUNDARY_VALUES {
        assert_eq!(L::new(value).load(), value);
        lock.store(value);
        assert_eq!(lock.load(), value);
    }
}

#[cfg(feature = "client")]
#[test]
fn atomic_static_derive_preserves_i32_boundaries() {
    assert_boundary_storage::<<BoundaryEnum as egui_states::AtomicStatic>::Lock>();
}

#[cfg(feature = "client")]
#[test]
fn atomic_derive_preserves_i32_boundaries() {
    use egui_states::{AtomicLock, AtomicLockStatic};

    type Lock = <BoundaryEnum as egui_states::Atomic>::Lock;
    assert_boundary_storage::<Lock>();
    let lock = Lock::new(BoundaryEnum::Max);
    let mut previous = BoundaryEnum::Max;
    for value in BOUNDARY_VALUES {
        let mut notified = false;
        lock.update(value, || {
            assert_eq!(lock.load(), previous);
            notified = true;
        });
        assert!(notified);
        assert_eq!(lock.load(), value);
        previous = value;
    }
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
