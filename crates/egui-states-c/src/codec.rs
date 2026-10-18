//! Writers and readers: the encoder and decoder of every value.

use std::ffi::{c_char, c_void};

use egui_states::c_abi::{Error, ErrorCode, Reader, Writer};

use crate::ffi::{
    boolean, data_kind, egui_states_status_t, elements, ffi, free, mutable, output,
    output_elements, text, write,
};
use crate::meta::{egui_states_data_type_t, egui_states_value_kind_t};

/// Reusable encoder for one value. Create it with `egui_states_writer_new`.
///
/// Write values in declaration order: struct fields and tuple elements one
/// after another, `Option` as `egui_states_write_none` or
/// `egui_states_write_some` followed by the value, `Vec` and `HashMap` as
/// `egui_states_write_len` followed by the elements or key/value pairs, and
/// enums as their variant index. Every function that takes a writer checks
/// its writes against the target type and leaves the writer unchanged; call
/// `egui_states_writer_clear` to reuse it.
pub struct egui_states_writer_t(pub(crate) Writer);

/// Reusable, type-checked decoder for one value. Create it with
/// `egui_states_reader_new`; getters and events load it.
///
/// Read values in the order they are written. Each read must match the type
/// of the next value; a failed read leaves the reader unchanged. Strings read
/// from a reader stay valid until it is loaded again or freed.
pub struct egui_states_reader_t(pub(crate) Reader);

fn writer_mut<'a>(writer: *mut egui_states_writer_t) -> Result<&'a mut Writer, Error> {
    Ok(&mut unsafe { mutable(writer, "writer") }?.0)
}

fn reader_mut<'a>(reader: *mut egui_states_reader_t) -> Result<&'a mut Reader, Error> {
    Ok(&mut unsafe { mutable(reader, "reader") }?.0)
}

fn read<T>(
    handle: *mut egui_states_reader_t,
    out: *mut T,
    read: impl FnOnce(&mut Reader) -> Result<T, Error>,
) -> egui_states_status_t {
    ffi(|| {
        output(out, "out_value")?;
        let value = read(reader_mut(handle)?)?;
        unsafe { write(out, value, "out_value") }
    })
}

// writer ----------------------------------------------------------------------

/// Creates an empty writer. Free it with `egui_states_writer_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_writer_new(
    out_writer: *mut *mut egui_states_writer_t,
) -> egui_states_status_t {
    ffi(|| {
        output(out_writer, "out_writer")?;
        let handle = Box::into_raw(Box::new(egui_states_writer_t(Writer::new())));
        unsafe { write(out_writer, handle, "out_writer") }
    })
}

/// Frees a writer. Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_writer_free(writer: *mut egui_states_writer_t) {
    unsafe { free(writer) }
}

/// Removes everything written so far.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_writer_clear(
    writer: *mut egui_states_writer_t,
) -> egui_states_status_t {
    ffi(|| {
        writer_mut(writer)?.clear();
        Ok(())
    })
}

/// Writes a `u8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_u8(
    writer: *mut egui_states_writer_t,
    value: u8,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.u8(value))
}

/// Writes a `u16`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_u16(
    writer: *mut egui_states_writer_t,
    value: u16,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.u16(value))
}

/// Writes a `u32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_u32(
    writer: *mut egui_states_writer_t,
    value: u32,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.u32(value))
}

/// Writes a `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_u64(
    writer: *mut egui_states_writer_t,
    value: u64,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.u64(value))
}

/// Writes an `i8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_i8(
    writer: *mut egui_states_writer_t,
    value: i8,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.i8(value))
}

/// Writes an `i16`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_i16(
    writer: *mut egui_states_writer_t,
    value: i16,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.i16(value))
}

/// Writes an `i32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_i32(
    writer: *mut egui_states_writer_t,
    value: i32,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.i32(value))
}

/// Writes an `i64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_i64(
    writer: *mut egui_states_writer_t,
    value: i64,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.i64(value))
}

/// Writes an `f32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_f32(
    writer: *mut egui_states_writer_t,
    value: f32,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.f32(value))
}

/// Writes an `f64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_f64(
    writer: *mut egui_states_writer_t,
    value: f64,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.f64(value))
}

/// Writes a `bool`; `value` must be 0 or 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_bool(
    writer: *mut egui_states_writer_t,
    value: u8,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.bool(boolean(value, "value")?))
}

/// Writes a UTF-8 `String` of `len` bytes, or a NUL-terminated one when `len`
/// is `EGUI_STATES_NUL_TERMINATED`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_string(
    writer: *mut egui_states_writer_t,
    data: *const c_char,
    len: usize,
) -> egui_states_status_t {
    ffi(|| {
        let value = unsafe { text(data, len, "data") }?;
        writer_mut(writer)?.string(value)
    })
}

/// Writes an absent `Option`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_none(
    writer: *mut egui_states_writer_t,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.none())
}

/// Writes a present `Option`; write its value next.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_some(
    writer: *mut egui_states_writer_t,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.some())
}

/// Writes the element count of a `Vec` or the entry count of a `HashMap`;
/// write the elements or key/value pairs next.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_len(
    writer: *mut egui_states_writer_t,
    count: usize,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.count(count as u64))
}

/// Writes an enum as its variant index in declaration order (not its
/// discriminant).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_enum(
    writer: *mut egui_states_writer_t,
    variant_index: u32,
) -> egui_states_status_t {
    ffi(|| writer_mut(writer)?.enum_index(variant_index))
}

/// Writes `count` native-endian numbers of `data_type` from `data`, as if each
/// was written on its own; for example the elements of a `Vec<f32>` or a
/// `[u8; 4]`. `data` may be null when `count` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_write_array(
    writer: *mut egui_states_writer_t,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
) -> egui_states_status_t {
    ffi(|| {
        let kind = data_kind(data_type)?;
        let raw = unsafe { elements(data, count, kind.size(), "data") }?;
        writer_mut(writer)?.array(kind, raw, count)
    })
}

// reader ----------------------------------------------------------------------

/// Creates an empty reader. Free it with `egui_states_reader_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_reader_new(
    out_reader: *mut *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| {
        output(out_reader, "out_reader")?;
        let handle = Box::into_raw(Box::new(egui_states_reader_t(Reader::new())));
        unsafe { write(out_reader, handle, "out_reader") }
    })
}

/// Frees a reader. Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_reader_free(reader: *mut egui_states_reader_t) {
    unsafe { free(reader) }
}

/// Reads a `u8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_u8(
    reader: *mut egui_states_reader_t,
    out_value: *mut u8,
) -> egui_states_status_t {
    read(reader, out_value, Reader::u8)
}

/// Reads a `u16`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_u16(
    reader: *mut egui_states_reader_t,
    out_value: *mut u16,
) -> egui_states_status_t {
    read(reader, out_value, Reader::u16)
}

/// Reads a `u32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_u32(
    reader: *mut egui_states_reader_t,
    out_value: *mut u32,
) -> egui_states_status_t {
    read(reader, out_value, Reader::u32)
}

/// Reads a `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_u64(
    reader: *mut egui_states_reader_t,
    out_value: *mut u64,
) -> egui_states_status_t {
    read(reader, out_value, Reader::u64)
}

/// Reads an `i8`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_i8(
    reader: *mut egui_states_reader_t,
    out_value: *mut i8,
) -> egui_states_status_t {
    read(reader, out_value, Reader::i8)
}

/// Reads an `i16`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_i16(
    reader: *mut egui_states_reader_t,
    out_value: *mut i16,
) -> egui_states_status_t {
    read(reader, out_value, Reader::i16)
}

/// Reads an `i32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_i32(
    reader: *mut egui_states_reader_t,
    out_value: *mut i32,
) -> egui_states_status_t {
    read(reader, out_value, Reader::i32)
}

/// Reads an `i64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_i64(
    reader: *mut egui_states_reader_t,
    out_value: *mut i64,
) -> egui_states_status_t {
    read(reader, out_value, Reader::i64)
}

/// Reads an `f32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_f32(
    reader: *mut egui_states_reader_t,
    out_value: *mut f32,
) -> egui_states_status_t {
    read(reader, out_value, Reader::f32)
}

/// Reads an `f64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_f64(
    reader: *mut egui_states_reader_t,
    out_value: *mut f64,
) -> egui_states_status_t {
    read(reader, out_value, Reader::f64)
}

/// Reads a `bool`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_bool(
    reader: *mut egui_states_reader_t,
    out_value: *mut bool,
) -> egui_states_status_t {
    read(reader, out_value, Reader::bool)
}

/// Reads a `String` as `len` UTF-8 bytes at `data`, borrowed from the reader
/// and not NUL-terminated. They stay valid until the reader is loaded again
/// or freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_string(
    reader: *mut egui_states_reader_t,
    out_data: *mut *const c_char,
    out_len: *mut usize,
) -> egui_states_status_t {
    ffi(|| {
        output(out_data, "out_data")?;
        output(out_len, "out_len")?;
        let value = reader_mut(reader)?.string()?;
        unsafe {
            out_data.write(value.as_ptr().cast());
            out_len.write(value.len());
        }
        Ok(())
    })
}

/// Reads whether an `Option` is present; when it is, its value follows.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_option(
    reader: *mut egui_states_reader_t,
    out_is_some: *mut bool,
) -> egui_states_status_t {
    read(reader, out_is_some, Reader::option)
}

/// Reads the element count of a `Vec` or the entry count of a `HashMap`; the
/// elements or key/value pairs follow.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_len(
    reader: *mut egui_states_reader_t,
    out_count: *mut usize,
) -> egui_states_status_t {
    read(reader, out_count, |reader| {
        let len = reader.count()?;
        usize::try_from(len).map_err(|_| {
            Error::new(
                ErrorCode::OutOfRange,
                format!("{len} elements exceed size_t"),
            )
        })
    })
}

/// Reads an enum as its variant index in declaration order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_enum(
    reader: *mut egui_states_reader_t,
    out_variant_index: *mut u32,
) -> egui_states_status_t {
    read(reader, out_variant_index, Reader::enum_index)
}

/// Reads `count` numbers of `data_type` into `out` as native-endian values,
/// as if each was read on its own. `out` may be null when `count` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_read_array(
    reader: *mut egui_states_reader_t,
    data_type: egui_states_data_type_t,
    out: *mut c_void,
    count: usize,
) -> egui_states_status_t {
    ffi(|| {
        let kind = data_kind(data_type)?;
        let mut empty = [];
        let out = unsafe { output_elements(out, count, kind.size(), "out") }?.unwrap_or(&mut empty);
        reader_mut(reader)?.array(kind, out, count)
    })
}

/// Reports the kind of the next value (`EGUI_STATES_KIND_*`), or
/// `EGUI_STATES_KIND_END` when every value has been read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_reader_peek(
    reader: *mut egui_states_reader_t,
    out_kind: *mut egui_states_value_kind_t,
) -> egui_states_status_t {
    read(reader, out_kind, |reader| Ok(reader.peek() as u32))
}

/// Skips the next value, including everything it contains.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_reader_skip(
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| reader_mut(reader)?.skip())
}

/// Reports whether every value has been read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_reader_at_end(
    reader: *mut egui_states_reader_t,
    out_at_end: *mut bool,
) -> egui_states_status_t {
    read(reader, out_at_end, |reader| Ok(reader.at_end()))
}
