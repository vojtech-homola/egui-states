use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::MaybeUninit;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub enum Object {
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
    Object(Vec<Object>),
    Empty,
}

impl Object {
    pub fn get_type(&self) -> ObjectType {
        match self {
            Object::U8(_) => ObjectType::U8,
            Object::U16(_) => ObjectType::U16,
            Object::U32(_) => ObjectType::U32,
            Object::U64(_) => ObjectType::U64,
            Object::I8(_) => ObjectType::I8,
            Object::I16(_) => ObjectType::I16,
            Object::I32(_) => ObjectType::I32,
            Object::I64(_) => ObjectType::I64,
            Object::F64(_) => ObjectType::F64,
            Object::F32(_) => ObjectType::F32,
            Object::String(_) => ObjectType::String,
            Object::Bool(_) => ObjectType::Bool,
            Object::Object(vec) => {
                ObjectType::Object(vec.iter().map(|obj| obj.get_type()).collect())
            }
            Object::Empty => ObjectType::Empty,
        }
    }
}

#[derive(Hash)]
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
    Object(Vec<ObjectType>),
    Empty,
}

impl ObjectType {
    pub fn get_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

// Conversion traits --------------------------------------------------------
// ToTransport
pub trait ToObject {
    fn as_object(&self) -> Object;
}

macro_rules! impl_base_to_transport {
    ($($t:ty, $variant:ident),*) => {
        $(
            impl ToObject for $t {
                #[inline]
                fn as_object(&self) -> Object {
                    Object::$variant(*self)
                }
            }
        )*
    };
}

impl_base_to_transport! {
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
    bool, Bool
}

impl ToObject for String {
    #[inline]
    fn as_object(&self) -> Object {
        Object::String(self.clone())
    }
}

impl ToObject for () {
    #[inline]
    fn as_object(&self) -> Object {
        Object::Empty
    }
}

macro_rules! impl_to_object_tuple {
    ($($idx:tt: $T:ident),*) => {
        impl<$($T),*> ToObject for ($($T,)*)
        where
            $($T: ToObject,)*
        {
            #[inline]
            fn as_object(&self) -> Object {
                Object::Object(vec![
                    $(self.$idx.as_object(),)*
                ])
            }
        }
    };
}

impl_to_object_tuple!(0: T0);
impl_to_object_tuple!(0: T0, 1: T1);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8);
impl_to_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9);

impl<T, const N: usize> ToObject for [T; N]
where
    T: ToObject,
{
    #[inline]
    fn as_object(&self) -> Object {
        Object::Object(self.iter().map(|item| item.as_object()).collect())
    }
}

// FromObject
pub trait FromObject: Sized {
    fn from_object(obj: &Object) -> Option<Self>;
}

macro_rules! impl_base_from_object {
    ($($t:ty, $variant:ident),*) => {
        $(
            impl FromObject for $t {
                #[inline]
                fn from_object(obj: &Object) -> Option<Self> {
                    if let Object::$variant(value) = obj {
                        Some(*value)
                    } else {
                        None
                    }
                }
            }
        )*
    };
}

impl_base_from_object! {
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
    bool, Bool
}

impl FromObject for String {
    #[inline]
    fn from_object(obj: &Object) -> Option<Self> {
        if let Object::String(value) = obj {
            Some(value.clone())
        } else {
            None
        }
    }
}

impl FromObject for () {
    #[inline]
    fn from_object(obj: &Object) -> Option<Self> {
        if let Object::Empty = obj {
            Some(())
        } else {
            None
        }
    }
}

macro_rules! impl_from_object_tuple {
    ($($idx:tt: $T:ident),*) => {
        impl<$($T),*> FromObject for ($($T,)*)
        where
            $($T: FromObject,)*
        {
            #[inline]
            fn from_object(obj: &Object) -> Option<Self> {
                if let Object::Object(vec) = obj {
                    Some((
                        $($T::from_object(&vec[$idx])?,)*
                    ))
                } else {
                    None
                }
            }
        }
    };
}

impl_from_object_tuple!(0: T0);
impl_from_object_tuple!(0: T0, 1: T1);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8);
impl_from_object_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9);

impl<T, const N: usize> FromObject for [T; N]
where
    T: FromObject,
{
    #[inline]
    fn from_object(obj: &Object) -> Option<Self> {
        if let Object::Object(vec) = obj {
            if vec.len() != N {
                return None;
            }
            let mut array: [MaybeUninit<T>; N] = unsafe { MaybeUninit::uninit().assume_init() };
            for (i, item) in vec.iter().enumerate() {
                array[i] = MaybeUninit::new(T::from_object(item)?);
            }
            let result = unsafe { std::mem::transmute_copy::<_, [T; N]>(&array) };
            Some(result)
        } else {
            None
        }
    }
}
