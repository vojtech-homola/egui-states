//! Protocol-level type descriptions used to validate peer compatibility.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::hashing::StableHasher;

#[derive(Clone, Debug, PartialEq)]
/// Runtime description of a value supported by the synchronization protocol.
pub enum ObjectType {
    /// An unsigned 8-bit integer.
    U8,
    /// An unsigned 16-bit integer.
    U16,
    /// An unsigned 32-bit integer.
    U32,
    /// An unsigned 64-bit integer.
    U64,
    /// A signed 8-bit integer.
    I8,
    /// A signed 16-bit integer.
    I16,
    /// A signed 32-bit integer.
    I32,
    /// A signed 64-bit integer.
    I64,
    /// A 64-bit floating-point number.
    F64,
    /// A 32-bit floating-point number.
    F32,
    /// A UTF-8 string.
    String,
    /// A Boolean value.
    Bool,
    /// A named fieldless enum and its `(variant, discriminant)` pairs.
    Enum(String, Vec<(String, i32)>),
    /// A named struct and its ordered `(field, type)` pairs.
    Struct(String, Vec<(String, ObjectType)>),
    /// A heterogeneous tuple.
    Tuple(Vec<ObjectType>),
    /// A fixed-size array, represented by its length and element type.
    List(u32, Box<ObjectType>),
    /// A variable-length vector.
    Vec(Box<ObjectType>),
    /// A map, represented by its key and value types.
    Map(Box<ObjectType>, Box<ObjectType>),
    /// An optional value.
    Option(Box<ObjectType>),
    /// The unit type, used for empty signals and take values.
    Empty,
}

impl Hash for ObjectType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            ObjectType::U8 => 0u8.hash(state),
            ObjectType::U16 => 1u8.hash(state),
            ObjectType::U32 => 2u8.hash(state),
            ObjectType::U64 => 3u8.hash(state),
            ObjectType::I8 => 4u8.hash(state),
            ObjectType::I16 => 5u8.hash(state),
            ObjectType::I32 => 6u8.hash(state),
            ObjectType::I64 => 7u8.hash(state),
            ObjectType::F64 => 8u8.hash(state),
            ObjectType::F32 => 9u8.hash(state),
            ObjectType::String => 10u8.hash(state),
            ObjectType::Bool => 11u8.hash(state),
            ObjectType::Enum(_, variants) => {
                12u8.hash(state);
                (variants.len() as u64).hash(state);
                for (variant_name, value) in variants {
                    variant_name.hash(state);
                    value.hash(state);
                }
            }
            ObjectType::Struct(_, fields) => {
                13u8.hash(state);
                (fields.len() as u64).hash(state);
                for (_, field_type) in fields {
                    field_type.hash(state);
                }
            }
            ObjectType::Tuple(types) => {
                14u8.hash(state);
                (types.len() as u64).hash(state);
                for value in types {
                    value.hash(state);
                }
            }
            ObjectType::List(size, inner) => {
                15u8.hash(state);
                size.hash(state);
                inner.hash(state);
            }
            ObjectType::Vec(inner) => {
                16u8.hash(state);
                inner.hash(state);
            }
            ObjectType::Map(key, value) => {
                17u8.hash(state);
                key.hash(state);
                value.hash(state);
            }
            ObjectType::Option(inner) => {
                18u8.hash(state);
                inner.hash(state);
            }
            ObjectType::Empty => 19u8.hash(state),
        }
    }
}

impl ObjectType {
    /// Returns the stable, platform-independent protocol hash for this type.
    ///
    /// The hash uses fixed discriminants and the declared order of tuple,
    /// struct, and enum members. A change to the serialized shape therefore
    /// changes the compatibility hash.
    pub fn get_hash(&self) -> u32 {
        let mut hasher = StableHasher::new();
        self.hash(&mut hasher);
        hasher.finish_u32()
    }

    /// Extends an existing stable hash with this type description.
    pub fn get_hash_from(&self, hash: u32) -> u32 {
        let mut hasher = StableHasher::new();
        hash.hash(&mut hasher);
        self.hash(&mut hasher);
        hasher.finish_u32()
    }
}

/// Formats `object_type` in the Rust type syntax accepted by the C ABI.
///
/// Named structs and enums are referenced by name; [`type_definition`]
/// formats their definitions.
#[cfg(feature = "c_abi")]
pub(crate) fn type_expression(object_type: &ObjectType) -> String {
    let mut output = String::new();
    write_type_expression(object_type, &mut output);
    output
}

#[cfg(feature = "c_abi")]
fn write_type_expression(object_type: &ObjectType, output: &mut String) {
    match object_type {
        ObjectType::U8 => output.push_str("u8"),
        ObjectType::U16 => output.push_str("u16"),
        ObjectType::U32 => output.push_str("u32"),
        ObjectType::U64 => output.push_str("u64"),
        ObjectType::I8 => output.push_str("i8"),
        ObjectType::I16 => output.push_str("i16"),
        ObjectType::I32 => output.push_str("i32"),
        ObjectType::I64 => output.push_str("i64"),
        ObjectType::F32 => output.push_str("f32"),
        ObjectType::F64 => output.push_str("f64"),
        ObjectType::String => output.push_str("String"),
        ObjectType::Bool => output.push_str("bool"),
        ObjectType::Empty => output.push_str("()"),
        ObjectType::Enum(name, _) | ObjectType::Struct(name, _) => output.push_str(name),
        // A zero-element tuple has no Rust spelling of its own and prints as
        // `()`, which parses back as `Empty`; Rust never produces one.
        ObjectType::Tuple(elements) => {
            output.push('(');
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                write_type_expression(element, output);
            }
            if elements.len() == 1 {
                output.push(',');
            }
            output.push(')');
        }
        ObjectType::List(size, element) => {
            output.push('[');
            write_type_expression(element, output);
            output.push_str("; ");
            output.push_str(&size.to_string());
            output.push(']');
        }
        ObjectType::Vec(element) => {
            output.push_str("Vec<");
            write_type_expression(element, output);
            output.push('>');
        }
        ObjectType::Map(key, value) => {
            output.push_str("HashMap<");
            write_type_expression(key, output);
            output.push_str(", ");
            write_type_expression(value, output);
            output.push('>');
        }
        ObjectType::Option(inner) => {
            output.push_str("Option<");
            write_type_expression(inner, output);
            output.push('>');
        }
    }
}

/// Formats the definition of a named struct or enum, or returns `None` for
/// other types.
///
/// Enum discriminants are always written explicitly.
#[cfg(feature = "c_abi")]
pub(crate) fn type_definition(object_type: &ObjectType) -> Option<String> {
    match object_type {
        ObjectType::Struct(name, fields) if fields.is_empty() => {
            Some(format!("struct {name} {{}}"))
        }
        ObjectType::Struct(name, fields) => {
            let fields = fields
                .iter()
                .map(|(field, field_type)| format!("{field}: {}", type_expression(field_type)))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("struct {name} {{ {fields} }}"))
        }
        ObjectType::Enum(name, variants) if variants.is_empty() => {
            Some(format!("enum {name} {{}}"))
        }
        ObjectType::Enum(name, variants) => {
            let variants = variants
                .iter()
                .map(|(variant, value)| format!("{variant} = {value}"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("enum {name} {{ {variants} }}"))
        }
        _ => None,
    }
}

/// Rust derives requested for a custom type emitted by the Rust server generator.
///
/// This is public because [`crate::typed`] emits it in downstream crates. It is
/// build metadata only and does not contribute to protocol compatibility hashes.
#[doc(hidden)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustDerive {
    /// Additional derives for a generated fieldless enum.
    Enum(&'static str, &'static [&'static str]),
    /// Additional derives for a generated struct.
    Struct(&'static str, &'static [&'static str]),
}

/// Describes how a Rust type is represented by the synchronization protocol.
///
/// Use [`egui_states::typed`](crate::typed) for user-defined structs and enums
/// instead of implementing this trait manually.
/// Implementations are provided for primitives, `String`, unit, `Option`,
/// tuples up to arity ten, arrays, vectors, and hash maps.
///
/// # Safety
///
/// [`Self::get_type`] must describe the exact Serde wire representation of
/// `Self`. Both peers use its hash to decide whether values are compatible;
/// an incorrect implementation can make otherwise incompatible serialized
/// values appear compatible.
pub unsafe trait Typed {
    /// Returns the protocol type description for `Self`.
    fn get_type() -> ObjectType;

    /// Returns Rust server code-generation metadata for custom types contained
    /// by `Self`.
    #[doc(hidden)]
    fn rust_derives() -> Vec<RustDerive> {
        Vec::new()
    }
}

macro_rules! impl_typed_base {
    ($(($type:ty, $type_variant:ident)),* $(,)?) => {
        $(
            unsafe impl Typed for $type {
                #[inline]
                fn get_type() -> ObjectType {
                    ObjectType::$type_variant
                }
            }
        )*
    };
}

impl_typed_base! {
    (bool, Bool),
    (u8, U8),
    (u16, U16),
    (u32, U32),
    (u64, U64),
    (i8, I8),
    (i16, I16),
    (i32, I32),
    (i64, I64),
    (f32, F32),
    (f64, F64)
}

unsafe impl Typed for String {
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::String
    }
}

unsafe impl Typed for () {
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Empty
    }
}

unsafe impl<T> Typed for Option<T>
where
    T: Typed,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Option(Box::new(T::get_type()))
    }

    fn rust_derives() -> Vec<RustDerive> {
        T::rust_derives()
    }
}

macro_rules! impl_typed_tuple {
    ($(($($idx:tt: $T:ident),*)),* $(,)?) => {
        $(
            unsafe impl<$($T),*> Typed for ($($T,)*)
            where
                $($T: Typed,)*
            {
                #[inline]
                fn get_type() -> ObjectType {
                    ObjectType::Tuple(vec![$($T::get_type()),*])
                }

                fn rust_derives() -> Vec<RustDerive> {
                    let mut derives = Vec::new();
                    $(derives.extend($T::rust_derives());)*
                    derives
                }
            }
        )*
    };
}

impl_typed_tuple! {
    (0: T0),
    (0: T0, 1: T1),
    (0: T0, 1: T1, 2: T2),
    (0: T0, 1: T1, 2: T2, 3: T3),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9)
}

unsafe impl<T, const N: usize> Typed for [T; N]
where
    T: Typed,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::List(N as u32, Box::new(T::get_type()))
    }

    fn rust_derives() -> Vec<RustDerive> {
        T::rust_derives()
    }
}

unsafe impl<T> Typed for Vec<T>
where
    T: Typed,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Vec(Box::new(T::get_type()))
    }

    fn rust_derives() -> Vec<RustDerive> {
        T::rust_derives()
    }
}

unsafe impl<K, V> Typed for HashMap<K, V>
where
    K: Typed,
    V: Typed,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Map(Box::new(K::get_type()), Box::new(V::get_type()))
    }

    fn rust_derives() -> Vec<RustDerive> {
        let mut derives = K::rust_derives();
        derives.extend(V::rust_derives());
        derives
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<ObjectType> {
        use ObjectType::*;
        vec![
            U8,
            U16,
            U32,
            U64,
            I8,
            I16,
            I32,
            I64,
            F64,
            F32,
            String,
            Bool,
            Empty,
            Enum("Choice".into(), vec![("A".into(), 0), ("B".into(), -1)]),
            Struct(
                "Point".into(),
                vec![("x".into(), I32), ("label".into(), String)],
            ),
            Tuple(vec![U8, Bool]),
            List(3, Box::new(U16)),
            Vec(Box::new(F64)),
            Map(Box::new(String), Box::new(U32)),
            Option(Box::new(I64)),
        ]
    }

    /// Type hashes are compared during every value update, between peers that may
    /// have been built from different releases.
    #[test]
    fn type_hashes_are_stable() {
        let actual: Vec<_> = samples().iter().map(ObjectType::get_hash).collect();
        assert_eq!(actual, GOLDEN_HASHES, "type hashes changed: {actual:?}");
        // Map states combine the key hash with the value hash.
        assert_eq!(
            ObjectType::U32.get_hash_from(ObjectType::U16.get_hash()),
            GOLDEN_MAP_HASH
        );
    }

    const GOLDEN_HASHES: [u32; 20] = [
        2617980014, 789771595, 3384066523, 149769992, 1352412645, 2593815527, 2557368423,
        1485256138, 2044193470, 791956523, 424131073, 2688995303, 3179252139, 3480834224,
        1621035420, 4132667259, 331490409, 679028502, 3527569321, 410823959,
    ];
    const GOLDEN_MAP_HASH: u32 = 2249675624;

    #[test]
    fn distinct_wire_shapes_have_distinct_hashes() {
        let hashes: std::collections::HashSet<_> =
            samples().iter().map(ObjectType::get_hash).collect();
        assert_eq!(hashes.len(), samples().len());
    }

    #[test]
    fn type_names_do_not_change_hashes_but_layout_does() {
        let point = |name: &str, field: &str| {
            ObjectType::Struct(name.into(), vec![(field.into(), ObjectType::I32)])
        };
        assert_eq!(point("A", "x").get_hash(), point("B", "y").get_hash());

        let choice = |name: &str, variant: &str, value: i32| {
            ObjectType::Enum(name.into(), vec![(variant.into(), value)])
        };
        assert_eq!(
            choice("A", "X", 1).get_hash(),
            choice("B", "X", 1).get_hash()
        );
        assert_ne!(
            choice("A", "X", 1).get_hash(),
            choice("A", "Y", 1).get_hash()
        );
        assert_ne!(
            choice("A", "X", 1).get_hash(),
            choice("A", "X", 2).get_hash()
        );

        let pair = |a, b| ObjectType::Tuple(vec![a, b]);
        assert_ne!(
            pair(ObjectType::U8, ObjectType::U16).get_hash(),
            pair(ObjectType::U16, ObjectType::U8).get_hash()
        );
        assert_ne!(
            ObjectType::List(2, Box::new(ObjectType::U8)).get_hash(),
            ObjectType::List(3, Box::new(ObjectType::U8)).get_hash()
        );
    }

    /// The C ABI parses these spellings and `generate_c` emits them, so they
    /// are part of the public C contract.
    #[test]
    #[cfg(feature = "c_abi")]
    fn type_syntax_is_rust_syntax() {
        let expressions: Vec<_> = samples().iter().map(type_expression).collect();
        assert_eq!(
            expressions,
            [
                "u8",
                "u16",
                "u32",
                "u64",
                "i8",
                "i16",
                "i32",
                "i64",
                "f64",
                "f32",
                "String",
                "bool",
                "()",
                "Choice",
                "Point",
                "(u8, bool)",
                "[u16; 3]",
                "Vec<f64>",
                "HashMap<String, u32>",
                "Option<i64>",
            ]
        );
        assert_eq!(
            type_expression(&ObjectType::Tuple(vec![ObjectType::Empty])),
            "((),)"
        );

        let definitions: Vec<_> = samples().iter().filter_map(type_definition).collect();
        assert_eq!(
            definitions,
            [
                "enum Choice { A = 0, B = -1 }",
                "struct Point { x: i32, label: String }"
            ]
        );
        assert_eq!(
            type_definition(&ObjectType::Struct("Unit".into(), Vec::new())).as_deref(),
            Some("struct Unit {}")
        );
    }
}
