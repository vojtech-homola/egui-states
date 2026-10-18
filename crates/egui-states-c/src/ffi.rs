//! Status codes, the last-error slot, panic containment, and argument checks.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

use egui_states::c_abi::{DataKind, Error, ErrorCode};

/// Result of every fallible function.
///
/// On failure, `egui_states_last_error` describes the error.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum egui_states_status_t {
    /// Success.
    EGUI_STATES_OK = 0,
    /// A null pointer, unknown flag bit, invalid enum value, invalid UTF-8,
    /// malformed type string, or malformed value data.
    EGUI_STATES_ERROR_INVALID_ARGUMENT = 1,
    /// The server is in the wrong lifecycle phase: registering after
    /// finalization, or operating or starting before it.
    EGUI_STATES_ERROR_INVALID_STATE = 2,
    /// An unknown state id, type name, map key, collection index, or missing
    /// previous value.
    EGUI_STATES_ERROR_NOT_FOUND = 3,
    /// Written or read values do not match the state's type, a data type
    /// differs from the registered one, or the id names another kind of state.
    EGUI_STATES_ERROR_TYPE_MISMATCH = 4,
    /// An index, range, size limit, or image rectangle is out of bounds.
    EGUI_STATES_ERROR_OUT_OF_RANGE = 5,
    /// An output buffer is too small; the required size was still reported.
    EGUI_STATES_ERROR_BUFFER_TOO_SMALL = 6,
    /// No event arrived before the timeout.
    EGUI_STATES_ERROR_TIMEOUT = 7,
    /// Event delivery was closed with `egui_states_server_close_events`.
    EGUI_STATES_ERROR_CLOSED = 8,
    /// A state or type definition with the same name already exists.
    EGUI_STATES_ERROR_ALREADY_EXISTS = 9,
    /// The server could not bind its socket or start its network thread.
    EGUI_STATES_ERROR_IO = 10,
    /// An unexpected internal failure.
    EGUI_STATES_ERROR_INTERNAL = 11,
    /// A panic was caught at the ABI boundary.
    EGUI_STATES_ERROR_PANIC = 12,
}

use egui_states_status_t::*;

fn status(code: ErrorCode) -> egui_states_status_t {
    match code {
        ErrorCode::InvalidArgument => EGUI_STATES_ERROR_INVALID_ARGUMENT,
        ErrorCode::InvalidState => EGUI_STATES_ERROR_INVALID_STATE,
        ErrorCode::NotFound => EGUI_STATES_ERROR_NOT_FOUND,
        ErrorCode::TypeMismatch => EGUI_STATES_ERROR_TYPE_MISMATCH,
        ErrorCode::OutOfRange => EGUI_STATES_ERROR_OUT_OF_RANGE,
        ErrorCode::BufferTooSmall => EGUI_STATES_ERROR_BUFFER_TOO_SMALL,
        ErrorCode::Timeout => EGUI_STATES_ERROR_TIMEOUT,
        ErrorCode::Closed => EGUI_STATES_ERROR_CLOSED,
        ErrorCode::AlreadyExists => EGUI_STATES_ERROR_ALREADY_EXISTS,
        ErrorCode::Io => EGUI_STATES_ERROR_IO,
        ErrorCode::Internal => EGUI_STATES_ERROR_INTERNAL,
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

pub(crate) fn set_last_error(message: &str) {
    let message = CString::new(message.replace('\0', "\\0")).unwrap_or_default();
    // `try_with` fails only during thread teardown, when nobody can read it.
    let _ = LAST_ERROR.try_with(|slot| *slot.borrow_mut() = message);
}

pub(crate) fn last_error() -> *const c_char {
    LAST_ERROR
        .try_with(|slot| slot.borrow().as_ptr())
        .unwrap_or(c"".as_ptr())
}

/// Runs one exported function: catches panics and records failures.
pub(crate) fn ffi(body: impl FnOnce() -> Result<(), Error>) -> egui_states_status_t {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => EGUI_STATES_OK,
        Ok(Err(error)) => {
            set_last_error(&error.message);
            status(error.code)
        }
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|text| text.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            set_last_error(&format!(
                "panic at the egui-states C ABI boundary: {detail}"
            ));
            EGUI_STATES_ERROR_PANIC
        }
    }
}

/// Drops a boxed handle, ignoring a panic from its destructor.
pub(crate) unsafe fn free<T>(handle: *mut T) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(handle) })));
    }
}

fn null(name: &str) -> Error {
    Error::new(
        ErrorCode::InvalidArgument,
        format!("`{name}` must not be null"),
    )
}

fn check_pointer<T>(pointer: *const T, name: &str) -> Result<(), Error> {
    if pointer.is_null() {
        return Err(null(name));
    }
    if !pointer.is_aligned() {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            format!("`{name}` is not aligned"),
        ));
    }
    Ok(())
}

/// Borrows a required handle or input.
pub(crate) unsafe fn reference<'a, T>(pointer: *const T, name: &str) -> Result<&'a T, Error> {
    check_pointer(pointer, name)?;
    Ok(unsafe { &*pointer })
}

/// Borrows a required handle mutably.
pub(crate) unsafe fn mutable<'a, T>(pointer: *mut T, name: &str) -> Result<&'a mut T, Error> {
    check_pointer(pointer, name)?;
    Ok(unsafe { &mut *pointer })
}

/// Borrows an optional handle; null is `None`.
pub(crate) unsafe fn optional<'a, T>(
    pointer: *const T,
    name: &str,
) -> Result<Option<&'a T>, Error> {
    if pointer.is_null() {
        return Ok(None);
    }
    Ok(Some(unsafe { reference(pointer, name) }?))
}

/// Borrows an optional handle mutably; null is `None`.
pub(crate) unsafe fn optional_mut<'a, T>(
    pointer: *mut T,
    name: &str,
) -> Result<Option<&'a mut T>, Error> {
    if pointer.is_null() {
        return Ok(None);
    }
    Ok(Some(unsafe { mutable(pointer, name) }?))
}

/// Checks a required output pointer before any work is done.
pub(crate) fn output<T>(pointer: *mut T, name: &str) -> Result<(), Error> {
    check_pointer(pointer, name)
}

/// Writes a required output.
pub(crate) unsafe fn write<T>(pointer: *mut T, value: T, name: &str) -> Result<(), Error> {
    check_pointer(pointer, name)?;
    unsafe { pointer.write(value) };
    Ok(())
}

/// Writes an optional output; null is skipped.
pub(crate) unsafe fn write_optional<T>(pointer: *mut T, value: T, name: &str) -> Result<(), Error> {
    if pointer.is_null() {
        return Ok(());
    }
    unsafe { write(pointer, value, name) }
}

/// Borrows a required NUL-terminated UTF-8 string.
pub(crate) unsafe fn c_str<'a>(pointer: *const c_char, name: &str) -> Result<&'a str, Error> {
    if pointer.is_null() {
        return Err(null(name));
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map_err(|_| Error::new(ErrorCode::InvalidArgument, format!("`{name}` is not UTF-8")))
}

/// Borrows an optional NUL-terminated UTF-8 string; null is `None`.
pub(crate) unsafe fn optional_c_str<'a>(
    pointer: *const c_char,
    name: &str,
) -> Result<Option<&'a str>, Error> {
    if pointer.is_null() {
        return Ok(None);
    }
    Ok(Some(unsafe { c_str(pointer, name) }?))
}

/// Borrows an input buffer. Any pointer, even null, is accepted when `len` is 0.
pub(crate) unsafe fn bytes<'a>(
    pointer: *const u8,
    len: usize,
    name: &str,
) -> Result<&'a [u8], Error> {
    if len == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err(null(name));
    }
    if len > isize::MAX as usize {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            format!("`{name}` is too large"),
        ));
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, len) })
}

/// Borrows `count` input elements of `size` bytes as bytes.
pub(crate) unsafe fn elements<'a>(
    pointer: *const std::ffi::c_void,
    count: usize,
    size: usize,
    name: &str,
) -> Result<&'a [u8], Error> {
    let len = byte_len(count, size, name)?;
    unsafe { bytes(pointer.cast(), len, name) }
}

/// Borrows an output buffer of `count` elements of `size` bytes.
///
/// A null pointer with a zero count asks only for the required size and is
/// `None`.
pub(crate) unsafe fn output_elements<'a>(
    pointer: *mut std::ffi::c_void,
    count: usize,
    size: usize,
    name: &str,
) -> Result<Option<&'a mut [u8]>, Error> {
    let len = byte_len(count, size, name)?;
    if pointer.is_null() {
        if count == 0 {
            return Ok(None);
        }
        return Err(null(name));
    }
    if len > isize::MAX as usize {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            format!("`{name}` is too large"),
        ));
    }
    Ok(Some(unsafe {
        std::slice::from_raw_parts_mut(pointer.cast::<u8>(), len)
    }))
}

fn byte_len(count: usize, size: usize, name: &str) -> Result<usize, Error> {
    count.checked_mul(size).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            format!("`{name}` element count overflows"),
        )
    })
}

/// Borrows a value string of `len` bytes, or NUL-terminated when `len` is
/// `EGUI_STATES_NUL_TERMINATED`.
pub(crate) unsafe fn text<'a>(
    pointer: *const c_char,
    len: usize,
    name: &str,
) -> Result<&'a str, Error> {
    if len == usize::MAX {
        return unsafe { c_str(pointer, name) };
    }
    let data = unsafe { bytes(pointer.cast(), len, name) }?;
    std::str::from_utf8(data)
        .map_err(|_| Error::new(ErrorCode::InvalidArgument, format!("`{name}` is not UTF-8")))
}

/// Copies `value` NUL-terminated into a caller buffer of `capacity` bytes.
///
/// The length without the NUL is always reported. A null buffer with zero
/// capacity asks only for the length.
pub(crate) unsafe fn copy_text(
    value: &str,
    buffer: *mut c_char,
    capacity: usize,
    out_len: *mut usize,
) -> Result<(), Error> {
    output(out_len, "out_len")?;
    let out = unsafe { output_elements(buffer.cast(), capacity, 1, "buffer") }?;
    unsafe { out_len.write(value.len()) };
    let Some(out) = out else {
        return Ok(());
    };
    if out.len() <= value.len() {
        return Err(Error::new(
            ErrorCode::BufferTooSmall,
            format!(
                "the buffer holds {} bytes, but {} are needed",
                out.len(),
                value.len() + 1
            ),
        ));
    }
    out[..value.len()].copy_from_slice(value.as_bytes());
    out[value.len()] = 0;
    Ok(())
}

pub(crate) fn too_small(capacity: usize, required: usize, unit: &str) -> Error {
    Error::new(
        ErrorCode::BufferTooSmall,
        format!("the buffer holds {capacity} {unit}, but {required} are needed"),
    )
}

/// Rejects flag bits outside `allowed`.
pub(crate) fn check_flags(value: u32, allowed: u32) -> Result<(), Error> {
    if value & !allowed != 0 {
        return Err(Error::new(
            ErrorCode::InvalidArgument,
            format!(
                "unsupported flag bits {:#x}; this function accepts {allowed:#x}",
                value & !allowed
            ),
        ));
    }
    Ok(())
}

pub(crate) fn data_kind(value: u32) -> Result<DataKind, Error> {
    DataKind::from_id(value).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            format!("{value} is not an EGUI_STATES_DATA_* value"),
        )
    })
}

pub(crate) fn boolean(value: u8, name: &str) -> Result<bool, Error> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new(
            ErrorCode::InvalidArgument,
            format!("`{name}` must be 0 or 1, got {value}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panics_become_a_status_and_a_message() {
        let status = ffi(|| panic!("boom"));
        assert_eq!(status, EGUI_STATES_ERROR_PANIC);
        let message = unsafe { CStr::from_ptr(last_error()) };
        assert_eq!(
            message.to_str().unwrap(),
            "panic at the egui-states C ABI boundary: boom"
        );
    }

    #[test]
    fn error_codes_map_to_equal_status_values() {
        for code in [
            ErrorCode::InvalidArgument,
            ErrorCode::InvalidState,
            ErrorCode::NotFound,
            ErrorCode::TypeMismatch,
            ErrorCode::OutOfRange,
            ErrorCode::BufferTooSmall,
            ErrorCode::Timeout,
            ErrorCode::Closed,
            ErrorCode::AlreadyExists,
            ErrorCode::Io,
            ErrorCode::Internal,
        ] {
            assert_eq!(status(code) as i32, code as i32);
        }
    }

    #[test]
    fn messages_keep_interior_nul_visible() {
        set_last_error("a\0b");
        let message = unsafe { CStr::from_ptr(last_error()) };
        assert_eq!(message.to_str().unwrap(), "a\\0b");
    }
}
