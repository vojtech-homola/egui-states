//! Numeric enumerations shared with the C header.

use crate::data_transport::DataType;

/// Element type of numeric primitives, `Data` states, and array reads and writes.
///
/// The values match the protocol data type ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum DataKind {
    /// `u8`.
    U8 = 0,
    /// `u16`.
    U16 = 1,
    /// `u32`.
    U32 = 2,
    /// `u64`.
    U64 = 3,
    /// `i8`.
    I8 = 4,
    /// `i16`.
    I16 = 5,
    /// `i32`.
    I32 = 6,
    /// `i64`.
    I64 = 7,
    /// `f32`.
    F32 = 8,
    /// `f64`.
    F64 = 9,
}

impl DataKind {
    /// Every kind in numeric order.
    pub const ALL: [Self; 10] = [
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::F32,
        Self::F64,
    ];

    /// Converts a C value, or returns `None` if it is not a valid kind.
    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.get(usize::try_from(id).ok()?).copied()
    }

    /// Size of one element in bytes.
    pub fn size(self) -> usize {
        self.data_type().item_size()
    }

    /// Rust spelling of the element type.
    pub fn name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }

    pub(crate) fn data_type(self) -> DataType {
        match self {
            Self::U8 => DataType::U8,
            Self::U16 => DataType::U16,
            Self::U32 => DataType::U32,
            Self::U64 => DataType::U64,
            Self::I8 => DataType::I8,
            Self::I16 => DataType::I16,
            Self::I32 => DataType::I32,
            Self::I64 => DataType::I64,
            Self::F32 => DataType::F32,
            Self::F64 => DataType::F64,
        }
    }
}

/// Kind of the next value a [`Reader`](super::Reader) expects.
///
/// Structs, tuples, fixed-size arrays, and `()` have no wire header of their
/// own; readers and writers step into their fields automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ValueKind {
    /// `u8`.
    U8 = 0,
    /// `u16`.
    U16 = 1,
    /// `u32`.
    U32 = 2,
    /// `u64`.
    U64 = 3,
    /// `i8`.
    I8 = 4,
    /// `i16`.
    I16 = 5,
    /// `i32`.
    I32 = 6,
    /// `i64`.
    I64 = 7,
    /// `f32`.
    F32 = 8,
    /// `f64`.
    F64 = 9,
    /// `bool`.
    Bool = 10,
    /// `String`.
    String = 11,
    /// `Option<T>`: a presence flag followed by the value when present.
    Option = 12,
    /// `Vec<T>`: an element count followed by the elements.
    Vec = 13,
    /// `HashMap<K, V>`: an entry count followed by keys and values.
    Map = 14,
    /// A fieldless enum, carried as its variant index.
    Enum = 15,
    /// No further value is expected.
    End = 16,
}

impl ValueKind {
    pub(crate) fn from_data_kind(kind: DataKind) -> Self {
        match kind {
            DataKind::U8 => Self::U8,
            DataKind::U16 => Self::U16,
            DataKind::U32 => Self::U32,
            DataKind::U64 => Self::U64,
            DataKind::I8 => Self::I8,
            DataKind::I16 => Self::I16,
            DataKind::I32 => Self::I32,
            DataKind::I64 => Self::I64,
            DataKind::F32 => Self::F32,
            DataKind::F64 => Self::F64,
        }
    }

    /// Human-readable name used in diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bool => "bool",
            Self::String => "String",
            Self::Option => "Option",
            Self::Vec => "Vec",
            Self::Map => "HashMap",
            Self::Enum => "enum",
            Self::End => "end of value",
        }
    }
}

/// Kind of a registered state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum StateKind {
    /// `Value<T>`.
    Value = 0,
    /// `Static<T>`.
    Static = 1,
    /// `ValueTake<T>`.
    ValueTake = 2,
    /// `Signal<T>`.
    Signal = 3,
    /// `VecState<T>`.
    Vec = 4,
    /// `MapState<K, V>`.
    Map = 5,
    /// `Image`.
    Image = 6,
    /// `ImageMulti`.
    ImageMulti = 7,
    /// `Data<T>`.
    Data = 8,
    /// `DataTake<T>`.
    DataTake = 9,
    /// `DataMulti<T>`.
    DataMulti = 10,
    /// `DataMultiTake<T>`.
    DataMultiTake = 11,
    /// A reserved server event: logging, connect, disconnect, or client message.
    System = 12,
}

impl StateKind {
    /// Human-readable name used in diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::Value => "Value",
            Self::Static => "Static",
            Self::ValueTake => "ValueTake",
            Self::Signal => "Signal",
            Self::Vec => "VecState",
            Self::Map => "MapState",
            Self::Image => "Image",
            Self::ImageMulti => "ImageMulti",
            Self::Data => "Data",
            Self::DataTake => "DataTake",
            Self::DataMulti => "DataMulti",
            Self::DataMultiTake => "DataMultiTake",
            Self::System => "system event",
        }
    }
}
