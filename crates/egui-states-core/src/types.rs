use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::hasher::StableHasher;

#[derive(Hash, Clone, PartialEq)]
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
    Enum(String, Vec<(String, isize)>),
    Struct(String, Vec<(String, ObjectType)>),
    Tuple(Vec<ObjectType>),
    List(u32, Box<ObjectType>),
    Vec(Box<ObjectType>),
    Map(Box<ObjectType>, Box<ObjectType>),
    Option(Box<ObjectType>),
    Empty,
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
