//! Versions, constants, and functions that need no server.

use std::ffi::c_char;

use egui_states::c_abi;

use crate::ffi::{c_str, egui_states_status_t, ffi, last_error, write};

/// Version of this C ABI. It changes when a released function changes.
pub const EGUI_STATES_ABI_VERSION: u32 = 1;

/// Timeout of `egui_states_server_wait_event` that waits without limit.
pub const EGUI_STATES_WAIT_INFINITE: u64 = u64::MAX;

/// String length meaning that the string is NUL-terminated.
pub const EGUI_STATES_NUL_TERMINATED: usize = usize::MAX;

/// Flag: ask the client to repaint after the change.
pub const EGUI_STATES_UPDATE: u32 = 1 << 0;
/// Flag of `egui_states_value_set`: also deliver the change to subscribers.
pub const EGUI_STATES_SIGNAL: u32 = 1 << 1;
/// Flag of take sets: the next take set waits until the client consumed this one.
pub const EGUI_STATES_BLOCKING: u32 = 1 << 2;
/// Flag of data take sets: keep the buffer for clients that connect later.
pub const EGUI_STATES_CACHE: u32 = 1 << 3;
/// Flag of image updates: the update may replace a pending update of the same region.
pub const EGUI_STATES_FORCE: u32 = 1 << 4;
/// Flag of `egui_states_server_add_value` and `egui_states_server_add_signal`:
/// deliver every change instead of coalescing pending changes.
pub const EGUI_STATES_QUEUE: u32 = 1 << 5;
/// Flag of `egui_states_server_subscribe`: Value events carry the replaced value.
pub const EGUI_STATES_PREVIOUS: u32 = 1 << 6;

/// Event delivery mode, selected with `egui_states_server_set_queue_mode`.
pub type egui_states_mode_t = u32;
/// Pending changes coalesce to the latest one.
pub const EGUI_STATES_MODE_SINGLE: egui_states_mode_t = 0;
/// Every change is delivered in order.
pub const EGUI_STATES_MODE_QUEUE: egui_states_mode_t = 1;

/// Id of the reserved system event for client log messages: `(u8, String)`,
/// where the `u8` level is 0 debug, 1 info, 2 warning, or 3 error.
pub const EGUI_STATES_ID_LOGGING: u64 = 0;
/// Id of the reserved system event for client connections: the peer address as `String`.
pub const EGUI_STATES_ID_ON_CONNECT: u64 = 1;
/// Id of the reserved system event for client disconnections: `()`.
pub const EGUI_STATES_ID_ON_DISCONNECT: u64 = 2;
/// Id of the reserved system event for diagnostic client messages: `String`.
pub const EGUI_STATES_ID_CLIENT_MESSAGE: u64 = 3;

/// Element type of numbers, data states, and array reads and writes.
pub type egui_states_data_type_t = u32;
/// `u8`.
pub const EGUI_STATES_DATA_U8: egui_states_data_type_t = 0;
/// `u16`.
pub const EGUI_STATES_DATA_U16: egui_states_data_type_t = 1;
/// `u32`.
pub const EGUI_STATES_DATA_U32: egui_states_data_type_t = 2;
/// `u64`.
pub const EGUI_STATES_DATA_U64: egui_states_data_type_t = 3;
/// `i8`.
pub const EGUI_STATES_DATA_I8: egui_states_data_type_t = 4;
/// `i16`.
pub const EGUI_STATES_DATA_I16: egui_states_data_type_t = 5;
/// `i32`.
pub const EGUI_STATES_DATA_I32: egui_states_data_type_t = 6;
/// `i64`.
pub const EGUI_STATES_DATA_I64: egui_states_data_type_t = 7;
/// `f32`.
pub const EGUI_STATES_DATA_F32: egui_states_data_type_t = 8;
/// `f64`.
pub const EGUI_STATES_DATA_F64: egui_states_data_type_t = 9;

/// Channel layout of image bytes.
pub type egui_states_image_format_t = u32;
/// One byte per pixel: grayscale intensity.
pub const EGUI_STATES_IMAGE_GRAY: egui_states_image_format_t = 0;
/// Two bytes per pixel: grayscale intensity and alpha.
pub const EGUI_STATES_IMAGE_GRAY_ALPHA: egui_states_image_format_t = 1;
/// Three bytes per pixel: red, green, and blue.
pub const EGUI_STATES_IMAGE_RGB: egui_states_image_format_t = 2;
/// Four bytes per pixel: red, green, blue, and alpha.
pub const EGUI_STATES_IMAGE_RGBA: egui_states_image_format_t = 3;

/// Kind of the next value of a reader, from `egui_states_reader_peek`.
/// Numbers use the values of `egui_states_data_type_t`.
pub type egui_states_value_kind_t = u32;
/// `u8`.
pub const EGUI_STATES_KIND_U8: egui_states_value_kind_t = 0;
/// `u16`.
pub const EGUI_STATES_KIND_U16: egui_states_value_kind_t = 1;
/// `u32`.
pub const EGUI_STATES_KIND_U32: egui_states_value_kind_t = 2;
/// `u64`.
pub const EGUI_STATES_KIND_U64: egui_states_value_kind_t = 3;
/// `i8`.
pub const EGUI_STATES_KIND_I8: egui_states_value_kind_t = 4;
/// `i16`.
pub const EGUI_STATES_KIND_I16: egui_states_value_kind_t = 5;
/// `i32`.
pub const EGUI_STATES_KIND_I32: egui_states_value_kind_t = 6;
/// `i64`.
pub const EGUI_STATES_KIND_I64: egui_states_value_kind_t = 7;
/// `f32`.
pub const EGUI_STATES_KIND_F32: egui_states_value_kind_t = 8;
/// `f64`.
pub const EGUI_STATES_KIND_F64: egui_states_value_kind_t = 9;
/// `bool`.
pub const EGUI_STATES_KIND_BOOL: egui_states_value_kind_t = 10;
/// `String`.
pub const EGUI_STATES_KIND_STRING: egui_states_value_kind_t = 11;
/// `Option<T>`: read the flag with `egui_states_read_option`.
pub const EGUI_STATES_KIND_OPTION: egui_states_value_kind_t = 12;
/// `Vec<T>`: read the count with `egui_states_read_len`.
pub const EGUI_STATES_KIND_VEC: egui_states_value_kind_t = 13;
/// `HashMap<K, V>`: read the count with `egui_states_read_len`.
pub const EGUI_STATES_KIND_MAP: egui_states_value_kind_t = 14;
/// An enum: read the variant index with `egui_states_read_enum`.
pub const EGUI_STATES_KIND_ENUM: egui_states_value_kind_t = 15;
/// Every value has been read.
pub const EGUI_STATES_KIND_END: egui_states_value_kind_t = 16;

/// Kind of a registered state, from `egui_states_server_state_kind`.
pub type egui_states_state_kind_t = u32;
/// `Value<T>`.
pub const EGUI_STATES_STATE_VALUE: egui_states_state_kind_t = 0;
/// `Static<T>`.
pub const EGUI_STATES_STATE_STATIC: egui_states_state_kind_t = 1;
/// `ValueTake<T>`.
pub const EGUI_STATES_STATE_VALUE_TAKE: egui_states_state_kind_t = 2;
/// `Signal<T>`.
pub const EGUI_STATES_STATE_SIGNAL: egui_states_state_kind_t = 3;
/// `VecState<T>`.
pub const EGUI_STATES_STATE_VEC: egui_states_state_kind_t = 4;
/// `MapState<K, V>`.
pub const EGUI_STATES_STATE_MAP: egui_states_state_kind_t = 5;
/// `Image`.
pub const EGUI_STATES_STATE_IMAGE: egui_states_state_kind_t = 6;
/// `ImageMulti`.
pub const EGUI_STATES_STATE_IMAGE_MULTI: egui_states_state_kind_t = 7;
/// `Data<T>`.
pub const EGUI_STATES_STATE_DATA: egui_states_state_kind_t = 8;
/// `DataTake<T>`.
pub const EGUI_STATES_STATE_DATA_TAKE: egui_states_state_kind_t = 9;
/// `DataMulti<T>`.
pub const EGUI_STATES_STATE_DATA_MULTI: egui_states_state_kind_t = 10;
/// `DataMultiTake<T>`.
pub const EGUI_STATES_STATE_DATA_MULTI_TAKE: egui_states_state_kind_t = 11;
/// A reserved system event (`EGUI_STATES_ID_*`).
pub const EGUI_STATES_STATE_SYSTEM: egui_states_state_kind_t = 12;

/// Returns `EGUI_STATES_ABI_VERSION` of the loaded library.
#[unsafe(no_mangle)]
pub extern "C" fn egui_states_abi_version() -> u32 {
    EGUI_STATES_ABI_VERSION
}

/// Returns the wire-protocol version; clients must use the same one.
#[unsafe(no_mangle)]
pub extern "C" fn egui_states_protocol_version() -> u32 {
    u32::from(c_abi::protocol_version())
}

/// Returns the library version as a static NUL-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn egui_states_library_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// Returns the message of the last failed call on this thread, or `""`.
///
/// The string stays valid until the next failing call on the same thread.
/// Successful calls leave it unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn egui_states_last_error() -> *const c_char {
    last_error()
}

/// Computes the protocol id of the state with the full dotted `name`, such as
/// `"root.settings.count"`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_state_id(
    name: *const c_char,
    out_id: *mut u64,
) -> egui_states_status_t {
    ffi(|| unsafe {
        let name = c_str(name, "name")?;
        write(out_id, c_abi::state_id(name), "out_id")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_states::c_abi::{DataKind, ImageFormat, StateKind, ValueKind};

    /// cbindgen only sees this crate, so the constants are literals here and
    /// must match the safe layer.
    #[test]
    fn constants_match_the_safe_layer() {
        assert_eq!(EGUI_STATES_ABI_VERSION, c_abi::ABI_VERSION);
        let data = [
            (EGUI_STATES_DATA_U8, DataKind::U8),
            (EGUI_STATES_DATA_U16, DataKind::U16),
            (EGUI_STATES_DATA_U32, DataKind::U32),
            (EGUI_STATES_DATA_U64, DataKind::U64),
            (EGUI_STATES_DATA_I8, DataKind::I8),
            (EGUI_STATES_DATA_I16, DataKind::I16),
            (EGUI_STATES_DATA_I32, DataKind::I32),
            (EGUI_STATES_DATA_I64, DataKind::I64),
            (EGUI_STATES_DATA_F32, DataKind::F32),
            (EGUI_STATES_DATA_F64, DataKind::F64),
        ];
        for (value, kind) in data {
            assert_eq!(value, kind as u32);
            assert_eq!(DataKind::from_id(value), Some(kind));
        }
        for (value, kind) in [
            (EGUI_STATES_KIND_U8, ValueKind::U8),
            (EGUI_STATES_KIND_U16, ValueKind::U16),
            (EGUI_STATES_KIND_U32, ValueKind::U32),
            (EGUI_STATES_KIND_U64, ValueKind::U64),
            (EGUI_STATES_KIND_I8, ValueKind::I8),
            (EGUI_STATES_KIND_I16, ValueKind::I16),
            (EGUI_STATES_KIND_I32, ValueKind::I32),
            (EGUI_STATES_KIND_I64, ValueKind::I64),
            (EGUI_STATES_KIND_F32, ValueKind::F32),
            (EGUI_STATES_KIND_F64, ValueKind::F64),
            (EGUI_STATES_KIND_BOOL, ValueKind::Bool),
            (EGUI_STATES_KIND_STRING, ValueKind::String),
            (EGUI_STATES_KIND_OPTION, ValueKind::Option),
            (EGUI_STATES_KIND_VEC, ValueKind::Vec),
            (EGUI_STATES_KIND_MAP, ValueKind::Map),
            (EGUI_STATES_KIND_ENUM, ValueKind::Enum),
            (EGUI_STATES_KIND_END, ValueKind::End),
        ] {
            assert_eq!(value, kind as u32);
        }
        for (value, kind) in [
            (EGUI_STATES_STATE_VALUE, StateKind::Value),
            (EGUI_STATES_STATE_STATIC, StateKind::Static),
            (EGUI_STATES_STATE_VALUE_TAKE, StateKind::ValueTake),
            (EGUI_STATES_STATE_SIGNAL, StateKind::Signal),
            (EGUI_STATES_STATE_VEC, StateKind::Vec),
            (EGUI_STATES_STATE_MAP, StateKind::Map),
            (EGUI_STATES_STATE_IMAGE, StateKind::Image),
            (EGUI_STATES_STATE_IMAGE_MULTI, StateKind::ImageMulti),
            (EGUI_STATES_STATE_DATA, StateKind::Data),
            (EGUI_STATES_STATE_DATA_TAKE, StateKind::DataTake),
            (EGUI_STATES_STATE_DATA_MULTI, StateKind::DataMulti),
            (EGUI_STATES_STATE_DATA_MULTI_TAKE, StateKind::DataMultiTake),
            (EGUI_STATES_STATE_SYSTEM, StateKind::System),
        ] {
            assert_eq!(value, kind as u32);
        }
        for (value, format) in [
            (EGUI_STATES_IMAGE_GRAY, ImageFormat::Gray),
            (EGUI_STATES_IMAGE_GRAY_ALPHA, ImageFormat::GrayAlpha),
            (EGUI_STATES_IMAGE_RGB, ImageFormat::Rgb),
            (EGUI_STATES_IMAGE_RGBA, ImageFormat::Rgba),
        ] {
            assert_eq!(ImageFormat::from_id(value), Some(format));
        }
        assert_eq!(EGUI_STATES_ID_LOGGING, c_abi::LOGGING_ID);
        assert_eq!(EGUI_STATES_ID_ON_CONNECT, c_abi::ON_CONNECT_ID);
        assert_eq!(EGUI_STATES_ID_ON_DISCONNECT, c_abi::ON_DISCONNECT_ID);
        assert_eq!(EGUI_STATES_ID_CLIENT_MESSAGE, c_abi::CLIENT_MESSAGE_ID);
    }
}
