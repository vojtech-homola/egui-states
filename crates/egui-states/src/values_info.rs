#[derive(Clone)]
pub enum ValueType {
    Value(String, TypeInfo, InitValue),
    Static(String, TypeInfo, InitValue),
    Image(String),
    Dict(String, TypeInfo, TypeInfo),
    List(String, TypeInfo),
    Graphs(String, TypeInfo),
    Signal(String, TypeInfo),
    SubState(String, &'static str),
}

#[derive(Clone, PartialEq)]
pub enum TypeInfo {
    Basic(&'static str),
    Tuple(Vec<TypeInfo>),
    Array(Box<TypeInfo>, usize),
    Option(Box<TypeInfo>),
    Struct(&'static str, Vec<(&'static str, TypeInfo)>),
    Enum(&'static str, Vec<(&'static str, isize)>),
}

#[derive(Clone)]
pub enum InitValue {
    Value(String),
    Option(Option<Box<InitValue>>),
    Struct(&'static str, Vec<(&'static str, InitValue)>),
    Tuple(Vec<InitValue>),
    Array(Vec<InitValue>),
}

pub trait GetTypeInfo {
    fn type_info() -> TypeInfo;
}

pub trait GetInitValue {
    fn init_value(&self) -> InitValue;
}

// basic types ---------------------------------------
macro_rules! impl_type_name {
    ($($type:ty => $name:literal),* $(,)?) => {
        $(
            impl GetTypeInfo for $type {
                #[inline]
                fn type_info() -> TypeInfo {
                    TypeInfo::Basic($name)
                }
            }
        )*
    };
}

impl_type_name! {
    String => "String",
    bool => "bool",
    u8 => "u8",
    u16 => "u16",
    u32 => "u32",
    u64 => "u64",
    i8 => "i8",
    i16 => "i16",
    i32 => "i32",
    i64 => "i64",
    f32 => "f32",
    f64 => "f64",
    () => "()",
}

macro_rules! impl_init_value {
    ($($type:ty),* $(,)?) => {
        $(
            impl GetInitValue for $type {
                #[inline]
                fn init_value(&self) -> InitValue {
                    InitValue::Value(format!("{:?}", self))
                }
            }
        )*
    };
}

impl_init_value!(bool, u8, u16, u32, u64, i8, i16, i32, i64, f32, f64);

impl GetInitValue for String {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Value(format!("{:?}.into()", self))
    }
}

// Option ----------------------------------------------
impl<T: GetTypeInfo> GetTypeInfo for Option<T> {
    #[inline]
    fn type_info() -> TypeInfo {
        TypeInfo::Option(Box::new(T::type_info()))
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
macro_rules! impl_tuple_type_name {
    ($(($($T:ident),*)),* $(,)?) => {
        $(
            impl<$($T: GetTypeInfo),*> GetTypeInfo for ($($T,)*) {
                #[inline]
                fn type_info() -> TypeInfo {
                    TypeInfo::Tuple(vec![$($T::type_info()),*])
                }
            }
        )*
    };
}

impl_tuple_type_name! {
    (T0),
    (T0, T1),
    (T0, T1, T2),
    (T0, T1, T2, T3),
    (T0, T1, T2, T3, T4),
    (T0, T1, T2, T3, T4, T5),
    (T0, T1, T2, T3, T4, T5, T6),
    (T0, T1, T2, T3, T4, T5, T6, T7),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8),
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9),
}

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
impl<T: GetTypeInfo, const N: usize> GetTypeInfo for [T; N] {
    #[inline]
    fn type_info() -> TypeInfo {
        TypeInfo::Array(Box::new(T::type_info()), N)
    }
}

impl<T: GetInitValue, const N: usize> GetInitValue for [T; N] {
    #[inline]
    fn init_value(&self) -> InitValue {
        InitValue::Array(self.iter().map(|v| v.init_value()).collect())
    }
}
