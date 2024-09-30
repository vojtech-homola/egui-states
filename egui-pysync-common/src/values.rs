use std::io::Read;
use std::net::TcpStream;

use crate::transport::{self, ParseError, MESS_SIZE, SIZE_START};

/*
Values and Signals

common head:
|1B - type | 4B - u32 value id | 1B - signal / update | = 6B

value head:
| HEAD SIZE - 6B - rest of the message |
*/

pub trait ReadValue: Sized + Send + Sync + Clone {
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String>;
}

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

pub enum ValueMessage {
    I64(i64),
    Double(f64),
    String(String),
    U64(u64),
    TwoF32([f32; 2]),
    TwoF64([f64; 2]),
    Bool(bool),
    Empty(()),
    General(Box<dyn WriteValue>),
}

impl ValueMessage {
    pub fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            ValueMessage::I64(v) => v.write_message(head),
            ValueMessage::Double(v) => v.write_message(head),
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

// basic values
macro_rules! impl_basic_two_value {
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

impl_basic_two_value!(f32, 4, 8, TwoF32);
impl_basic_two_value!(f64, 8, 16, TwoF64);

// String -----------------------------------------------------
impl WriteValue for String {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        let size = self.len();
        head[SIZE_START..].copy_from_slice(&(size as u64).to_le_bytes());

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

// bool -----------------------------------------------------
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

// Empty -----------------------------------------------------
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
