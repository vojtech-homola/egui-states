use pyo3::buffer::{Element, PyUntypedBuffer};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::protocol::data::DataType;
use crate::server_core::data_core::DataSlice;

/// Gets the buffer of a Data argument.
pub(crate) fn data_buffer(data: &Bound<PyAny>) -> PyResult<PyUntypedBuffer> {
    PyUntypedBuffer::get(data)
        .map_err(|_| PyValueError::new_err("Data must be a bytes-like object."))
}

/// Checks `buffer` against a Data state's element type and views its bytes.
pub(crate) fn data_slice(buffer: &PyUntypedBuffer, data_type: DataType) -> PyResult<DataSlice<'_>> {
    check_data_type(buffer, data_type).map_err(PyValueError::new_err)?;
    // SAFETY: a C-contiguous buffer holds `len_bytes()` bytes from
    // `buf_ptr()`, and `buffer` keeps them alive while the view is borrowed.
    let bytes =
        unsafe { std::slice::from_raw_parts(buffer.buf_ptr().cast::<u8>(), buffer.len_bytes()) };
    Ok(DataSlice { bytes, data_type })
}

fn check_data_type(buffer: &PyUntypedBuffer, data_type: DataType) -> Result<(), String> {
    if !buffer.is_c_contiguous() {
        return Err("Data buffer must be C-contiguous.".to_string());
    }

    let format = buffer.format();
    // pyo3's format check ignores the byte-order prefix, and the bytes are copied as-is.
    let non_native: &[u8] = if cfg!(target_endian = "little") {
        b">!"
    } else {
        b"<"
    };
    if format
        .to_bytes()
        .first()
        .is_some_and(|order| non_native.contains(order))
    {
        return Err(format!(
            "Data buffer must use native byte order, got format '{:?}'",
            format
        ));
    }

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
