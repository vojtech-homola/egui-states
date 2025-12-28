use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::hasher::StableHasher;

#[derive(Clone, PartialEq)]
pub enum ObjectType {
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
    Enum(String, Vec<(String, i32)>),
    Struct(String, Vec<(String, ObjectType)>),
    Tuple(Vec<ObjectType>),
    List(u32, Box<ObjectType>),
    Vec(Box<ObjectType>),
    Map(Box<ObjectType>, Box<ObjectType>),
    Option(Box<ObjectType>),
    Empty,
}

// Manual Hash implementation for cross-platform stability.
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
            ObjectType::Enum(name, variants) => {
                12u8.hash(state);
                name.hash(state);
                (variants.len() as u64).hash(state);
                for (variant_name, value) in variants {
                    variant_name.hash(state);
                    value.hash(state);
                }
            }
            ObjectType::Struct(name, fields) => {
                13u8.hash(state);
                name.hash(state);
                (fields.len() as u64).hash(state);
                for (field_name, field_type) in fields {
                    field_name.hash(state);
                    field_type.hash(state);
                }
            }
            ObjectType::Tuple(types) => {
                14u8.hash(state);
                (types.len() as u64).hash(state);
                for t in types {
                    t.hash(state);
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
    pub fn get_hash(&self) -> u64 {
        let mut hasher = StableHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

// Conversion traits --------------------------------------------------------
pub trait GetType {
    fn get_type() -> ObjectType;
}

macro_rules! impl_get_type_base {
    ($($t:ty, $variant:ident),*) => {
        $(
            impl GetType for $t {
                #[inline]
                fn get_type() -> ObjectType {
                    ObjectType::$variant
                }
            }
        )*
    };
}

impl_get_type_base! {
    u8, U8,
    u16, U16,
    u32, U32,
    u64, U64,
    i8, I8,
    i16, I16,
    i32, I32,
    i64, I64,
    f32, F32,
    f64, F64,
    bool, Bool,
    String, String,
    (), Empty
}

impl<T> GetType for Option<T>
where
    T: GetType,
{
    fn get_type() -> ObjectType {
        ObjectType::Option(Box::new(T::get_type()))
    }
}

macro_rules! impl_get_type_tuple {
    ($($T:ident),*) => {
        impl<$($T),*> GetType for ($($T,)*)
        where
            $($T: GetType,)*
        {
            #[inline]
            fn get_type() -> ObjectType {
                ObjectType::Tuple(vec![
                    $($T::get_type(),)*
                ])
            }
        }
    };
}

impl_get_type_tuple!(T0);
impl_get_type_tuple!(T0, T1);
impl_get_type_tuple!(T0, T1, T2);
impl_get_type_tuple!(T0, T1, T2, T3);
impl_get_type_tuple!(T0, T1, T2, T3, T4);
impl_get_type_tuple!(T0, T1, T2, T3, T4, T5);
impl_get_type_tuple!(T0, T1, T2, T3, T4, T5, T6);
impl_get_type_tuple!(T0, T1, T2, T3, T4, T5, T6, T7);
impl_get_type_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
impl_get_type_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);

impl<T, const N: usize> GetType for [T; N]
where
    T: GetType,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::List(N as u32, Box::new(T::get_type()))
    }
}

impl<T> GetType for Vec<T>
where
    T: GetType,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Vec(Box::new(T::get_type()))
    }
}

impl<K, V> GetType for HashMap<K, V>
where
    K: GetType,
    V: GetType,
{
    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Map(Box::new(K::get_type()), Box::new(V::get_type()))
    }
}
