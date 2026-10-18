//! Server lifecycle, type definitions, state registration, and introspection.

use std::ffi::c_char;
use std::net::Ipv4Addr;

use egui_states::c_abi::{CServer, Error, ErrorCode};

use crate::codec::egui_states_writer_t;
use crate::ffi::{
    c_str, check_flags, copy_text, data_kind, egui_states_status_t, ffi, free, optional,
    optional_c_str, output, reference, write, write_optional,
};
use crate::meta::{EGUI_STATES_QUEUE, egui_states_data_type_t, egui_states_state_kind_t};

/// A state server. Create it with `egui_states_server_new`.
///
/// Register type definitions and states, call `egui_states_server_finalize`,
/// subscribe to events, then call `egui_states_server_start`. Every function
/// taking a server is thread-safe, except `egui_states_server_free`.
pub struct egui_states_server_t(pub(crate) CServer);

pub(crate) fn server_ref<'a>(server: *const egui_states_server_t) -> Result<&'a CServer, Error> {
    Ok(&unsafe { reference(server, "server") }?.0)
}

fn optional_writer<'a>(
    writer: *const egui_states_writer_t,
    name: &str,
) -> Result<Option<&'a egui_states::c_abi::Writer>, Error> {
    Ok(unsafe { optional(writer, name) }?.map(|writer| &writer.0))
}

/// Registers a state with `register` and reports its id.
fn add(
    server: *const egui_states_server_t,
    name: *const c_char,
    out_id: *mut u64,
    register: impl FnOnce(&CServer, &str) -> Result<u64, Error>,
) -> egui_states_status_t {
    ffi(|| {
        let name = unsafe { c_str(name, "name") }?;
        let id = register(server_ref(server)?, name)?;
        unsafe { write_optional(out_id, id, "out_id") }
    })
}

/// Creates a server. With a non-null `version`, clients must present the same
/// application version during the handshake, for example a generated
/// `*_VERSION_HASH`. Free the server with `egui_states_server_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_new(
    version: *const u64,
    out_server: *mut *mut egui_states_server_t,
) -> egui_states_status_t {
    ffi(|| {
        output(out_server, "out_server")?;
        let version = unsafe { optional(version, "version") }?.copied();
        let handle = Box::into_raw(Box::new(egui_states_server_t(CServer::new(version))));
        unsafe { write(out_server, handle, "out_server") }
    })
}

/// Stops and frees a server. Null is ignored.
///
/// No other call may use the server concurrently: call
/// `egui_states_server_close_events`, wait for threads blocked in
/// `egui_states_server_wait_event` to return, then free it. Readers, writers,
/// and events stay valid afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_free(server: *mut egui_states_server_t) {
    unsafe { free(server) }
}

/// Registers a struct or enum definition for later type strings, such as
/// `"struct Point { x: i32, y: i32 }"` or `"enum Mode { Fast = 0, Slow = 1 }"`.
///
/// Types it uses must be defined first. Field and variant names, variant
/// order, and discriminants must match the Rust client. Repeating an
/// identical definition succeeds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_define_type(
    server: *const egui_states_server_t,
    definition: *const c_char,
) -> egui_states_status_t {
    ffi(|| {
        let definition = unsafe { c_str(definition, "definition") }?;
        server_ref(server)?.define_type(definition)
    })
}

/// Computes the protocol hash of a type string, as clients compare it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_type_hash(
    server: *const egui_states_server_t,
    type_text: *const c_char,
    out_hash: *mut u32,
) -> egui_states_status_t {
    ffi(|| {
        output(out_hash, "out_hash")?;
        let type_text = unsafe { c_str(type_text, "type_text") }?;
        let hash = server_ref(server)?.type_hash(type_text)?;
        unsafe { write(out_hash, hash, "out_hash") }
    })
}

/// Registers a `Value<T>` named `name` (the full dotted path, such as
/// `"root.count"`) with Rust type string `type_text`.
///
/// `initial` holds the initial value; null is an empty writer, valid for `()`.
/// `flags` may be `EGUI_STATES_QUEUE`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_value(
    server: *const egui_states_server_t,
    name: *const c_char,
    type_text: *const c_char,
    initial: *const egui_states_writer_t,
    flags: u32,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        check_flags(flags, EGUI_STATES_QUEUE)?;
        let type_text = unsafe { c_str(type_text, "type_text") }?;
        let initial = optional_writer(initial, "initial")?;
        server.add_value(name, type_text, initial, flags & EGUI_STATES_QUEUE != 0)
    })
}

/// Registers a `Static<T>` with its initial value; null `initial` is an empty
/// writer. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_static(
    server: *const egui_states_server_t,
    name: *const c_char,
    type_text: *const c_char,
    initial: *const egui_states_writer_t,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        let type_text = unsafe { c_str(type_text, "type_text") }?;
        let initial = optional_writer(initial, "initial")?;
        server.add_static(name, type_text, initial)
    })
}

/// Registers a `ValueTake<T>`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_value_take(
    server: *const egui_states_server_t,
    name: *const c_char,
    type_text: *const c_char,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        let type_text = unsafe { c_str(type_text, "type_text") }?;
        server.add_value_take(name, type_text)
    })
}

/// Registers a `Signal<T>`. `flags` may be `EGUI_STATES_QUEUE`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_signal(
    server: *const egui_states_server_t,
    name: *const c_char,
    type_text: *const c_char,
    flags: u32,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        check_flags(flags, EGUI_STATES_QUEUE)?;
        let type_text = unsafe { c_str(type_text, "type_text") }?;
        server.add_signal(name, type_text, flags & EGUI_STATES_QUEUE != 0)
    })
}

/// Registers a `VecState<T>` with element type `element_type`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_vec(
    server: *const egui_states_server_t,
    name: *const c_char,
    element_type: *const c_char,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        let element_type = unsafe { c_str(element_type, "element_type") }?;
        server.add_vec(name, element_type)
    })
}

/// Registers a `MapState<K, V>`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_map(
    server: *const egui_states_server_t,
    name: *const c_char,
    key_type: *const c_char,
    value_type: *const c_char,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        let key_type = unsafe { c_str(key_type, "key_type") }?;
        let value_type = unsafe { c_str(value_type, "value_type") }?;
        server.add_map(name, key_type, value_type)
    })
}

/// Registers an `Image`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_image(
    server: *const egui_states_server_t,
    name: *const c_char,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| server.add_image(name))
}

/// Registers an `ImageMulti`. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_image_multi(
    server: *const egui_states_server_t,
    name: *const c_char,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        server.add_image_multi(name)
    })
}

/// Registers a `Data<T>` of `EGUI_STATES_DATA_*` elements. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_data(
    server: *const egui_states_server_t,
    name: *const c_char,
    data_type: egui_states_data_type_t,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        server.add_data(name, data_kind(data_type)?)
    })
}

/// Registers a `DataTake<T>` of `EGUI_STATES_DATA_*` elements. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_data_take(
    server: *const egui_states_server_t,
    name: *const c_char,
    data_type: egui_states_data_type_t,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        server.add_data_take(name, data_kind(data_type)?)
    })
}

/// Registers a `DataMulti<T>` of `EGUI_STATES_DATA_*` elements. `out_id` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_data_multi(
    server: *const egui_states_server_t,
    name: *const c_char,
    data_type: egui_states_data_type_t,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        server.add_data_multi(name, data_kind(data_type)?)
    })
}

/// Registers a `DataMultiTake<T>` of `EGUI_STATES_DATA_*` elements. `out_id`
/// may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_add_data_multi_take(
    server: *const egui_states_server_t,
    name: *const c_char,
    data_type: egui_states_data_type_t,
    out_id: *mut u64,
) -> egui_states_status_t {
    add(server, name, out_id, |server, name| {
        server.add_data_multi_take(name, data_kind(data_type)?)
    })
}

/// Ends registration. Every other operation needs a finalized server; calling
/// it again succeeds without effect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_finalize(
    server: *const egui_states_server_t,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.finalize())
}

/// Starts listening on `port`.
///
/// `ipv4` is a dotted address such as `"127.0.0.1"`; null listens on every
/// IPv4 interface. A non-null `token` requires clients to present it. Starting
/// a running server succeeds without changing its settings; a stopped server
/// can be started again with other settings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_start(
    server: *const egui_states_server_t,
    port: u16,
    ipv4: *const c_char,
    token: *const c_char,
) -> egui_states_status_t {
    ffi(|| {
        let ip = match unsafe { optional_c_str(ipv4, "ipv4") }? {
            Some(text) => Some(text.parse::<Ipv4Addr>().map_err(|_| {
                Error::new(
                    ErrorCode::InvalidArgument,
                    format!("`{text}` is not an IPv4 address"),
                )
            })?),
            None => None,
        };
        let token = unsafe { optional_c_str(token, "token") }?.map(str::to_owned);
        server_ref(server)?.start(port, ip, token)
    })
}

/// Stops listening and disconnects the client.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_stop(
    server: *const egui_states_server_t,
) -> egui_states_status_t {
    ffi(|| {
        server_ref(server)?.stop();
        Ok(())
    })
}

/// Disconnects the client and keeps listening.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_disconnect_client(
    server: *const egui_states_server_t,
) -> egui_states_status_t {
    ffi(|| {
        server_ref(server)?.disconnect_client();
        Ok(())
    })
}

/// Reports whether the server is listening.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_is_running(
    server: *const egui_states_server_t,
    out_running: *mut bool,
) -> egui_states_status_t {
    ffi(|| unsafe { write(out_running, server_ref(server)?.is_running(), "out_running") })
}

/// Reports whether a client is connected.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_is_connected(
    server: *const egui_states_server_t,
    out_connected: *mut bool,
) -> egui_states_status_t {
    ffi(|| unsafe {
        write(
            out_connected,
            server_ref(server)?.is_connected(),
            "out_connected",
        )
    })
}

/// Asks the client to repaint after `delay_seconds`, or immediately for 0.
/// Without a client it does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_update(
    server: *const egui_states_server_t,
    delay_seconds: f32,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.update(delay_seconds))
}

/// Reports the kind (`EGUI_STATES_STATE_*`) of the state with `id`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_state_kind(
    server: *const egui_states_server_t,
    id: u64,
    out_kind: *mut egui_states_state_kind_t,
) -> egui_states_status_t {
    ffi(|| {
        output(out_kind, "out_kind")?;
        let kind = server_ref(server)?.state_kind(id)?;
        unsafe { write(out_kind, kind as u32, "out_kind") }
    })
}

/// Copies the full name of the state with `id`, NUL-terminated, into
/// `buffer` of `capacity` bytes.
///
/// `out_len` receives the length without the NUL even when the buffer is too
/// small. A null buffer with zero capacity asks only for the length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_state_name(
    server: *const egui_states_server_t,
    id: u64,
    buffer: *mut c_char,
    capacity: usize,
    out_len: *mut usize,
) -> egui_states_status_t {
    ffi(|| {
        let name = server_ref(server)?.state_name(id)?;
        unsafe { copy_text(&name, buffer, capacity, out_len) }
    })
}

/// Copies the type string of the state with `id` like
/// `egui_states_server_state_name`: the element type of vectors and data
/// states, and `HashMap<K, V>` for maps. Images have none (`NOT_FOUND`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_state_type(
    server: *const egui_states_server_t,
    id: u64,
    buffer: *mut c_char,
    capacity: usize,
    out_len: *mut usize,
) -> egui_states_status_t {
    ffi(|| {
        let type_text = server_ref(server)?.state_type(id)?;
        unsafe { copy_text(&type_text, buffer, capacity, out_len) }
    })
}
