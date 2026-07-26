use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
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

pub unsafe trait InitialValue {
    fn init_value(&self) -> InitValue;
}

macro_rules! impl_initial_value_base {
    ($(($type:ty, $variant:ident)),* $(,)?) => {
        $(
            unsafe impl InitialValue for $type {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::$variant(*self)
                }
            }
        )*
    };
}

impl_initial_value_base! {
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

unsafe impl InitialValue for String {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::String(self.clone())
    }
}

unsafe impl InitialValue for () {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Tuple(Vec::new())
    }
}

unsafe impl<T> InitialValue for Option<T>
where
    T: InitialValue,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        match self {
            Some(value) => InitValue::Option(Some(Box::new(value.init_value()))),
            None => InitValue::Option(None),
        }
    }
}

macro_rules! impl_initial_value_tuple {
    ($(($($idx:tt: $T:ident),*)),* $(,)?) => {
        $(
            unsafe impl<$($T),*> InitialValue for ($($T,)*)
            where
                $($T: InitialValue,)*
            {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::Tuple(vec![$(self.$idx.init_value()),*])
                }
            }
        )*
    };
}

impl_initial_value_tuple! {
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

unsafe impl<T, const N: usize> InitialValue for [T; N]
where
    T: InitialValue,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::List(self.iter().map(InitialValue::init_value).collect())
    }
}

unsafe impl<T> InitialValue for Vec<T>
where
    T: InitialValue,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Vec(self.iter().map(InitialValue::init_value).collect())
    }
}

unsafe impl<K, V> InitialValue for HashMap<K, V>
where
    K: InitialValue,
    V: InitialValue,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Map(
            self.iter()
                .map(|(key, value)| (key.init_value(), value.init_value()))
                .collect(),
        )
    }
}
