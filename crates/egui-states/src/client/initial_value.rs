use std::collections::HashMap;

#[derive(Clone, Debug)]
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

impl PartialEq for InitValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::U8(left), Self::U8(right)) => left == right,
            (Self::U16(left), Self::U16(right)) => left == right,
            (Self::U32(left), Self::U32(right)) => left == right,
            (Self::U64(left), Self::U64(right)) => left == right,
            (Self::I8(left), Self::I8(right)) => left == right,
            (Self::I16(left), Self::I16(right)) => left == right,
            (Self::I32(left), Self::I32(right)) => left == right,
            (Self::I64(left), Self::I64(right)) => left == right,
            (Self::F64(left), Self::F64(right)) => {
                left.is_nan() && right.is_nan() || left.to_bits() == right.to_bits()
            }
            (Self::F32(left), Self::F32(right)) => {
                left.is_nan() && right.is_nan() || left.to_bits() == right.to_bits()
            }
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Enum(left), Self::Enum(right)) => left == right,
            (Self::Option(left), Self::Option(right)) => left == right,
            (Self::Struct(left_name, left), Self::Struct(right_name, right)) => {
                left_name == right_name && left == right
            }
            (Self::Tuple(left), Self::Tuple(right)) => left == right,
            (Self::List(left), Self::List(right)) => left == right,
            (Self::Vec(left), Self::Vec(right)) => left == right,
            (Self::Map(left), Self::Map(right)) => left == right,
            _ => false,
        }
    }
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

/// Sort key used to canonicalize `InitValue::Map` entries.
///
/// Every integer variant shares a rank so keys order numerically rather than
/// lexicographically; all keys of one map have the same variant, so collapsing
/// them is safe. Shapes that cannot be a `HashMap` key in practice fall back to
/// their `Debug` rendering, which is deterministic for any given value.
///
/// This is deliberately not an `Ord` impl: `Ord` requires `Eq`, and `PartialEq`
/// above treats every NaN payload as equal while a float ordering cannot.
fn map_sort_key(value: &InitValue) -> (u8, i128, String) {
    match value {
        InitValue::Bool(value) => (0, i128::from(*value), String::new()),
        InitValue::U8(value) => (1, i128::from(*value), String::new()),
        InitValue::U16(value) => (1, i128::from(*value), String::new()),
        InitValue::U32(value) => (1, i128::from(*value), String::new()),
        InitValue::U64(value) => (1, i128::from(*value), String::new()),
        InitValue::I8(value) => (1, i128::from(*value), String::new()),
        InitValue::I16(value) => (1, i128::from(*value), String::new()),
        InitValue::I32(value) => (1, i128::from(*value), String::new()),
        InitValue::I64(value) => (1, i128::from(*value), String::new()),
        InitValue::String(value) | InitValue::Enum(value) => (2, 0, value.clone()),
        other => (3, 0, format!("{other:?}")),
    }
}

unsafe impl<K, V> InitialValue for HashMap<K, V>
where
    K: InitialValue,
    V: InitialValue,
{
    #[inline]
    fn init_value(&self) -> InitValue {
        let mut entries = self
            .iter()
            .map(|(key, value)| (key.init_value(), value.init_value()))
            .collect::<Vec<_>>();
        // `HashMap` iteration order differs between instances, so canonicalize
        // it here. Generated bindings stay byte-stable across builds, and two
        // structurally identical maps compare equal -- which `StateType`
        // equality relies on to validate a state class used more than once.
        entries.sort_by_cached_key(|(key, _)| map_sort_key(key));
        InitValue::Map(entries)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{InitValue, InitialValue};

    fn map_keys(value: &InitValue) -> Vec<String> {
        let InitValue::Map(entries) = value else {
            panic!("expected a map, got {value:?}");
        };
        entries
            .iter()
            .map(|(key, _)| match key {
                InitValue::U16(key) => key.to_string(),
                InitValue::String(key) => key.clone(),
                other => panic!("unexpected key {other:?}"),
            })
            .collect()
    }

    #[test]
    fn structurally_equal_maps_compare_equal() {
        // Two `HashMap`s with the same contents iterate in different orders, so
        // without canonicalization the two `InitValue`s compare unequal and
        // `StateType` equality rejects a state class used more than once.
        let make = || HashMap::from([(1u16, 1u32), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6)]);

        for _ in 0..64 {
            assert_eq!(make().init_value(), make().init_value());
        }
    }

    #[test]
    fn map_entries_are_ordered_by_key() {
        let numeric = HashMap::from([(10u16, 0u32), (1, 0), (20, 0), (2, 0), (3, 0)]);
        assert_eq!(map_keys(&numeric.init_value()), ["1", "2", "3", "10", "20"]);

        let text = HashMap::from([("b".to_string(), 0u32), ("c".into(), 0), ("a".into(), 0)]);
        assert_eq!(map_keys(&text.init_value()), ["a", "b", "c"]);
    }

    #[test]
    fn equality_preserves_generated_float_definitions() {
        let left_nan = InitValue::F32(f32::from_bits(0x7fc0_0001));
        let right_nan = InitValue::F32(f32::from_bits(0xffc0_1234));

        assert_eq!(left_nan, right_nan);
        assert_ne!(InitValue::F64(0.0), InitValue::F64(-0.0));
    }

    #[test]
    fn equality_applies_float_rules_recursively() {
        let left = InitValue::Option(Some(Box::new(InitValue::Vec(vec![InitValue::F64(
            f64::from_bits(0x7ff8_0000_0000_0001),
        )]))));
        let right = InitValue::Option(Some(Box::new(InitValue::Vec(vec![InitValue::F64(
            f64::from_bits(0xfff8_0000_0000_1234),
        )]))));

        assert_eq!(left, right);
    }
}
