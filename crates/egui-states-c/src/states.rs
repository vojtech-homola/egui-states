//! Values, statics, takes, signals, vectors, and maps.

use egui_states::c_abi::{Error, Reader, Writer};

use crate::codec::{egui_states_reader_t, egui_states_writer_t};
use crate::ffi::{
    check_flags, egui_states_status_t, ffi, mutable, optional, optional_mut, output, reference,
    write,
};
use crate::meta::{EGUI_STATES_BLOCKING, EGUI_STATES_SIGNAL, EGUI_STATES_UPDATE};
use crate::server::{egui_states_server_t, server_ref};

pub(crate) fn writer_ref<'a>(
    writer: *const egui_states_writer_t,
    name: &str,
) -> Result<&'a Writer, Error> {
    Ok(&unsafe { reference(writer, name) }?.0)
}

pub(crate) fn optional_writer<'a>(
    writer: *const egui_states_writer_t,
) -> Result<Option<&'a Writer>, Error> {
    Ok(unsafe { optional(writer, "writer") }?.map(|writer| &writer.0))
}

pub(crate) fn reader_mut<'a>(reader: *mut egui_states_reader_t) -> Result<&'a mut Reader, Error> {
    Ok(&mut unsafe { mutable(reader, "reader") }?.0)
}

fn optional_reader<'a>(reader: *mut egui_states_reader_t) -> Result<Option<&'a mut Reader>, Error> {
    Ok(unsafe { optional_mut(reader, "out_removed") }?.map(|reader| &mut reader.0))
}

/// Checks `value` against `allowed` and returns whether `flag` is set.
pub(crate) fn has(value: u32, allowed: u32, flag: u32) -> Result<bool, Error> {
    check_flags(value, allowed)?;
    Ok(value & flag != 0)
}

// values ----------------------------------------------------------------------

/// Loads the value of a `Value` state into `reader`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_value_get(
    server: *const egui_states_server_t,
    id: u64,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.value_get(id, reader_mut(reader)?))
}

/// Replaces the value of a `Value` state with the value in `writer`.
///
/// `flags` may combine `EGUI_STATES_UPDATE` and `EGUI_STATES_SIGNAL`, which
/// also delivers the change to subscribers of this server.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_value_set(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let allowed = EGUI_STATES_UPDATE | EGUI_STATES_SIGNAL;
        let signal = has(flags, allowed, EGUI_STATES_SIGNAL)?;
        let update = flags & EGUI_STATES_UPDATE != 0;
        server_ref(server)?.value_set(id, writer_ref(value, "value")?, signal, update)
    })
}

/// Loads the value of a `Static` state into `reader`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_static_get(
    server: *const egui_states_server_t,
    id: u64,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.static_get(id, reader_mut(reader)?))
}

/// Replaces the value of a `Static` state. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_static_set(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        server_ref(server)?.static_set(id, writer_ref(value, "value")?, update)
    })
}

/// Sends a one-shot value to a `ValueTake` state; null `value` is empty, as
/// for `()`.
///
/// `flags` may combine `EGUI_STATES_BLOCKING` and `EGUI_STATES_UPDATE`. While
/// a client is connected, this waits until the client consumed the previous
/// blocking transfer or disconnects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_value_take_set(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let allowed = EGUI_STATES_BLOCKING | EGUI_STATES_UPDATE;
        let blocking = has(flags, allowed, EGUI_STATES_BLOCKING)?;
        let update = flags & EGUI_STATES_UPDATE != 0;
        server_ref(server)?.value_take_set(id, optional_writer(value)?, blocking, update)
    })
}

/// Delivers a value of a `Signal` state to this server's subscribers, as if
/// the client sent it; null `value` is empty, as for `()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_signal_emit(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.signal_emit(id, optional_writer(value)?))
}

// vectors ---------------------------------------------------------------------

/// Loads a whole `VecState` into `reader` as a `Vec<T>`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_get(
    server: *const egui_states_server_t,
    id: u64,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.vec_get(id, reader_mut(reader)?))
}

/// Replaces a whole `VecState` with the `Vec<T>` in `value`: its length
/// followed by the elements. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_set(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        server_ref(server)?.vec_set(id, writer_ref(value, "value")?, update)
    })
}

/// Loads the element at `index` into `reader`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_get_item(
    server: *const egui_states_server_t,
    id: u64,
    index: usize,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.vec_get_item(id, index, reader_mut(reader)?))
}

/// Replaces the element at `index`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_set_item(
    server: *const egui_states_server_t,
    id: u64,
    index: usize,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        server_ref(server)?.vec_set_item(id, index, writer_ref(value, "value")?, update)
    })
}

/// Appends an element. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_append(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        server_ref(server)?.vec_append(id, writer_ref(value, "value")?, update)
    })
}

/// Removes the element at `index` and loads it into `out_removed` unless it
/// is null. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_remove(
    server: *const egui_states_server_t,
    id: u64,
    index: usize,
    flags: u32,
    out_removed: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        server_ref(server)?.vec_remove(id, index, update, optional_reader(out_removed)?)
    })
}

/// Reports the number of elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_vec_len(
    server: *const egui_states_server_t,
    id: u64,
    out_len: *mut usize,
) -> egui_states_status_t {
    ffi(|| {
        output(out_len, "out_len")?;
        let len = server_ref(server)?.vec_len(id)?;
        unsafe { write(out_len, len, "out_len") }
    })
}

// maps ------------------------------------------------------------------------

/// Loads a whole `MapState` into `reader` as a `HashMap<K, V>`, in
/// unspecified order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_map_get(
    server: *const egui_states_server_t,
    id: u64,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.map_get(id, reader_mut(reader)?))
}

/// Replaces a whole `MapState` with the `HashMap<K, V>` in `value`: its entry
/// count followed by keys and values. Keys must be unique. `flags` may be
/// `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_map_set(
    server: *const egui_states_server_t,
    id: u64,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        server_ref(server)?.map_set(id, writer_ref(value, "value")?, update)
    })
}

/// Loads the value stored under the key in `key` into `reader`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_map_get_item(
    server: *const egui_states_server_t,
    id: u64,
    key: *const egui_states_writer_t,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.map_get_item(id, writer_ref(key, "key")?, reader_mut(reader)?))
}

/// Inserts or replaces the value stored under `key`. `flags` may be
/// `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_map_set_item(
    server: *const egui_states_server_t,
    id: u64,
    key: *const egui_states_writer_t,
    value: *const egui_states_writer_t,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        let key = writer_ref(key, "key")?;
        server_ref(server)?.map_set_item(id, key, writer_ref(value, "value")?, update)
    })
}

/// Removes the entry under `key` and loads its value into `out_removed`
/// unless it is null. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_map_remove(
    server: *const egui_states_server_t,
    id: u64,
    key: *const egui_states_writer_t,
    flags: u32,
    out_removed: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| {
        let update = has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)?;
        let key = writer_ref(key, "key")?;
        server_ref(server)?.map_remove(id, key, update, optional_reader(out_removed)?)
    })
}

/// Reports the number of entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_map_len(
    server: *const egui_states_server_t,
    id: u64,
    out_len: *mut usize,
) -> egui_states_status_t {
    ffi(|| {
        output(out_len, "out_len")?;
        let len = server_ref(server)?.map_len(id)?;
        unsafe { write(out_len, len, "out_len") }
    })
}
