use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::hashing::StableHasher;

#[derive(Clone)]
pub enum InitValue {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F64(f64),
    F32(f32),
    String(String),
    Bool(bool),
    Enum(String),
    Option(Option<Box<InitValue>>),
    Struct(&'static str, Vec<(&'static str, InitValue)>),
    Tuple(Vec<InitValue>),
    List(Vec<InitValue>),
    Vec(Vec<InitValue>),
    Map(Vec<(InitValue, InitValue)>),
}

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
    pub fn get_hash(&self) -> u32 {
        let mut hasher = StableHasher::new();
        self.hash(&mut hasher);
        hasher.finish_u32()
    }
}

pub unsafe trait Transportable {
    fn init_value(&self) -> InitValue;
    fn get_type() -> ObjectType;
}

macro_rules! impl_transportable_base {
    ($(($type:ty, $type_variant:ident, $init_variant:ident)),* $(,)?) => {
        $(
            unsafe impl Transportable for $type {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::$init_variant(*self)
                }

                #[inline]
                fn get_type() -> ObjectType {
                    ObjectType::$type_variant
                }
            }
        )*
    };
}

impl_transportable_base! {
    (bool, Bool, Bool),
    (u8, U8, U8),
    (u16, U16, U16),
    (u32, U32, U32),
    (u64, U64, U64),
    (i8, I8, I8),
    (i16, I16, I16),
    (i32, I32, I32),
    (i64, I64, I64),
    (f32, F32, F32),
    (f64, F64, F64)
}

unsafe impl Transportable for String {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::String(self.clone())
    }

    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::String
    }
}

unsafe impl Transportable for () {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Tuple(Vec::new())
    }

    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Empty
    }
}

unsafe impl<T> Transportable for Option<T>
where
    T: Transportable,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        match self {
            Some(value) => InitValue::Option(Some(Box::new(value.init_value()))),
            None => InitValue::Option(None),
        }
    }

    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Option(Box::new(T::get_type()))
    }
}

macro_rules! impl_transportable_tuple {
    ($(($($idx:tt: $T:ident),*)),* $(,)?) => {
        $(
            unsafe impl<$($T),*> Transportable for ($($T,)*)
            where
                $($T: Transportable,)*
            {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::Tuple(vec![$(self.$idx.init_value()),*])
                }

                #[inline]
                fn get_type() -> ObjectType {
                    ObjectType::Tuple(vec![$($T::get_type()),*])
                }
            }
        )*
    };
}

impl_transportable_tuple! {
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

unsafe impl<T, const N: usize> Transportable for [T; N]
where
    T: Transportable,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::List(self.iter().map(|value| value.init_value()).collect())
    }

    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::List(N as u32, Box::new(T::get_type()))
    }
}

unsafe impl<T> Transportable for Vec<T>
where
    T: Transportable,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Vec(self.iter().map(|value| value.init_value()).collect())
    }

    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Vec(Box::new(T::get_type()))
    }
}

unsafe impl<K, V> Transportable for HashMap<K, V>
where
    K: Transportable,
    V: Transportable,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Map(
            self.iter()
                .map(|(key, value)| (key.init_value(), value.init_value()))
                .collect(),
        )
    }

    #[inline]
    fn get_type() -> ObjectType {
        ObjectType::Map(Box::new(K::get_type()), Box::new(V::get_type()))
    }
}
