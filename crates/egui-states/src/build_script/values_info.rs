use std::collections::HashMap;

use egui_states_core::graphs::GraphType;
use egui_states_core::types::ObjectType;

#[derive(Clone)]
pub enum StateType {
    Value(String, ObjectType, InitValue, bool),
    Static(String, ObjectType, InitValue),
    Image(String),
    Map(String, ObjectType, ObjectType),
    List(String, ObjectType),
    Graphs(String, GraphType),
    Signal(String, ObjectType, bool),
    SubState(String, &'static str, Vec<StateType>),
}

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

pub trait GetInitValue {
    fn init_value(&self) -> InitValue;
}

// basic types ---------------------------------------
macro_rules! impl_init_value {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(
            impl GetInitValue for $type {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::$variant(*self)
                }
            }
        )*
    };
}

impl_init_value! {
    bool => Bool,
    u8 => U8,
    u16 => U16,
    u32 => U32,
    u64 => U64,
    i8 => I8,
    i16 => I16,
    i32 => I32,
    i64 => I64,
    f32 => F32,
    f64 => F64,
}

impl GetInitValue for String {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::String(self.clone())
    }
}

impl<T: GetInitValue> GetInitValue for Option<T> {
    #[inline]
    fn init_value(&self) -> InitValue {
        match self {
            Some(v) => InitValue::Option(Some(Box::new(v.init_value()))),
            None => InitValue::Option(None),
        }
    }
}

// tuples ----------------------------------------------
macro_rules! impl_tuple_init_value {
    ($(($($idx:tt: $T:ident),*)),* $(,)?) => {
        $(
            impl<$($T: GetInitValue),*> GetInitValue for ($($T,)*) {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::Tuple(vec![$(self.$idx.init_value()),*])
                }
            }
        )*
    };
}

impl_tuple_init_value! {
    (0: T0),
    (0: T0, 1: T1),
    (0: T0, 1: T1, 2: T2),
    (0: T0, 1: T1, 2: T2, 3: T3),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8),
    (0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9),
}

// arrays ----------------------------------------------
impl<T: GetInitValue, const N: usize> GetInitValue for [T; N] {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::List(self.iter().map(|v| v.init_value()).collect())
    }
}

// Vec ----------------------------------------------
impl<T: GetInitValue> GetInitValue for Vec<T> {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Vec(self.iter().map(|v| v.init_value()).collect())
    }
}

// Map ----------------------------------------------
impl<K: GetInitValue, V: GetInitValue> GetInitValue for HashMap<K, V> {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Map(
            self.iter()
                .map(|(k, v)| (k.init_value(), v.init_value()))
                .collect(),
        )
    }
}
