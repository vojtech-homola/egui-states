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
    /// Returns the stable protocol hash for this type description.
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

/// Describes how a Rust type is represented by the synchronization protocol.
///
/// Use [`egui_states::typed`](crate::typed) for user-defined structs and enums
/// instead of implementing this trait manually.
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
}

unsafe impl<T> Typed for Vec<T>
where
    T: Typed,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Vec(Box::new(T::get_type()))
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
}
