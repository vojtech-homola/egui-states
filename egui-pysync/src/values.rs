/*
Values and Signals
*/

pub trait WriteValue: Send + Sync + 'static {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>>;

    #[inline]
    fn into_message(self) -> ValueMessage
    where
        Self: Sized,
    {
        ValueMessage::General(Box::new(self))
    }
}

pub trait ReadValue: Sized + Send + Sync + Clone {
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String>;
}

pub enum ValueMessage {
    I64(i64),
    Double(f64),
    Float(f32),
    String(String),
    U64(u64),
    TwoF32([f32; 2]),
    TwoF64([f64; 2]),
    Bool(bool),
    Empty(()),
    General(Box<dyn WriteValue>),
}

impl ValueMessage {
    pub(crate) fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            ValueMessage::I64(v) => v.write_message(head),
            ValueMessage::Double(v) => v.write_message(head),
            ValueMessage::Float(v) => v.write_message(head),
            ValueMessage::String(v) => v.write_message(head),
            ValueMessage::U64(v) => v.write_message(head),
            ValueMessage::TwoF32(v) => v.write_message(head),
            ValueMessage::TwoF64(v) => v.write_message(head),
            ValueMessage::Bool(v) => v.write_message(head),
            ValueMessage::Empty(v) => v.write_message(head),
            ValueMessage::General(v) => v.write_message(head),
        }
    }
}

// -----------------------------------------------------
// basic values
macro_rules! impl_basic_value {
    ($type:ty, $size:literal, $enum_type:ident) => {
        impl WriteValue for $type {
            fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
                head[0..$size].copy_from_slice(&self.to_le_bytes());
                None
            }

            #[inline]
            fn into_message(self) -> ValueMessage {
                ValueMessage::$enum_type(self)
            }
        }

        impl ReadValue for $type {
            #[inline]
            fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
                if data.is_some() {
                    return Err(format!(
                        "Value {} do not accept additional data.",
                        stringify!($type)
                    ));
                }

                Ok(<$type>::from_le_bytes(head[0..$size].try_into().unwrap()))
            }
        }
    };
}

impl_basic_value!(i64, 8, I64);
impl_basic_value!(u64, 8, U64);
impl_basic_value!(f64, 8, Double);
impl_basic_value!(f32, 4, Float);

macro_rules! impl_basic_2_value {
    ($t:ty, $size_1:literal, $size_2:literal, $enum_type:ident) => {
        impl WriteValue for [$t; 2] {
            fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
                head[0..$size_1].copy_from_slice(&self[0].to_le_bytes());
                head[$size_1..$size_2].copy_from_slice(&self[1].to_le_bytes());
                None
            }

            #[inline]
            fn into_message(self) -> ValueMessage {
                ValueMessage::$enum_type(self)
            }
        }

        impl ReadValue for [$t; 2] {
            #[inline]
            fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
                if data.is_some() {
                    return Err(format!(
                        "Value {} do not accept additional data.",
                        stringify!([$t; 2])
                    ));
                }

                let r = [
                    <$t>::from_le_bytes(head[0..$size_1].try_into().unwrap()),
                    <$t>::from_le_bytes(head[$size_1..$size_2].try_into().unwrap()),
                ];
                Ok(r)
            }
        }
    };
}

impl_basic_2_value!(f32, 4, 8, TwoF32);
impl_basic_2_value!(f64, 8, 16, TwoF64);

// String
impl WriteValue for String {
    fn write_message(&self, _head: &mut [u8]) -> Option<Vec<u8>> {
        Some(self.as_bytes().to_vec())
    }

    #[inline]
    fn into_message(self) -> ValueMessage {
        ValueMessage::String(self)
    }
}

impl ReadValue for String {
    #[inline]
    fn read_message(_head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        match data {
            Some(data) => Ok(String::from_utf8(data).unwrap()),
            None => Err("String value needs additional data.".to_string()),
        }
    }
}

// bool
impl WriteValue for bool {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        head[0] = *self as u8;
        None
    }

    #[inline]
    fn into_message(self) -> ValueMessage {
        ValueMessage::Bool(self)
    }
}

impl ReadValue for bool {
    #[inline]
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if data.is_some() {
            return Err("Bool value do not accept additional data.".to_string());
        }

        Ok(head[0] != 0)
    }
}

// Empty
impl WriteValue for () {
    fn write_message(&self, _head: &mut [u8]) -> Option<Vec<u8>> {
        None
    }

    #[inline]
    fn into_message(self) -> ValueMessage {
        ValueMessage::Empty(())
    }
}

impl ReadValue for () {
    #[inline]
    fn read_message(_head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if data.is_some() {
            return Err("Empty value do not accept additional data.".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HEAD_SIZE;

    #[test]
    fn test_i64() {
        let value = 1234567890;
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = i64::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_u64() {
        let value = 1234567890;
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = u64::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_f64() {
        let value = 1234.5678;
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = f64::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_string() {
        let value = "Hello, World!".to_string();
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, Some(value.as_bytes().to_vec()));

        let new_value = String::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_bool() {
        let value = true;
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = bool::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_empty() {
        let value = ();
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = <()>::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_two_f32() {
        let value = [1234.5678, 8765.4321];
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = <[f32; 2]>::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }

    #[test]
    fn test_two_f64() {
        let value = [1234.5678, 8765.4321];
        let mut head = [0u8; HEAD_SIZE];
        let data = value.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let new_value = <[f64; 2]>::read_message(&head[6..], data).unwrap();
        assert_eq!(value, new_value);
    }
}
