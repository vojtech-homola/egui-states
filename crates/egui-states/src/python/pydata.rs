use pyo3::buffer::{Element, PyUntypedBuffer};

use crate::data_transport::DataType;

pub(crate) fn check_data_type(buffer: &PyUntypedBuffer, data_type: DataType) -> Result<(), String> {
    if !buffer.is_c_contiguous() {
        return Err("Data buffer must be C-contiguous.".to_string());
    }

    let format = buffer.format();
    let result = match data_type {
        DataType::F32 => f32::is_compatible_format(format),
        DataType::F64 => f64::is_compatible_format(format),
        DataType::U8 => u8::is_compatible_format(format),
        DataType::I8 => i8::is_compatible_format(format),
        DataType::U16 => u16::is_compatible_format(format),
        DataType::I16 => i16::is_compatible_format(format),
        DataType::U32 => u32::is_compatible_format(format),
        DataType::I32 => i32::is_compatible_format(format),
        DataType::U64 => u64::is_compatible_format(format),
        DataType::I64 => i64::is_compatible_format(format),
    };

    match result {
        true => Ok(()),
        false => Err(format!(
            "Data type mismatch: expected {:?}, got format '{:?}'",
            data_type, format
        )),
    }
}
