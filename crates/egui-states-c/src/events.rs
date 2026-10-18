//! Subscriptions and polled client events.

use std::time::{Duration, Instant};

use egui_states::c_abi::{ChangeEvent, Error, ErrorCode};

use crate::codec::egui_states_reader_t;
use crate::ffi::{egui_states_status_t, ffi, free, output, reference, write};
use crate::meta::{
    EGUI_STATES_MODE_QUEUE, EGUI_STATES_MODE_SINGLE, EGUI_STATES_PREVIOUS, egui_states_mode_t,
};
use crate::server::{egui_states_server_t, server_ref};
use crate::states::{has, reader_mut};

/// A claimed change of one subscribed state, from
/// `egui_states_server_wait_event`.
///
/// While it is alive, no other event of the same state is delivered; free it
/// with `egui_states_event_free` to receive the next one. It stays valid
/// after its server is freed.
pub struct egui_states_event_t(ChangeEvent);

fn event_ref<'a>(event: *const egui_states_event_t) -> Result<&'a ChangeEvent, Error> {
    Ok(&unsafe { reference(event, "event") }?.0)
}

/// Starts delivering events of a `Value`, `Signal`, or system state
/// (`EGUI_STATES_ID_*`) to `egui_states_server_wait_event`.
///
/// `Value` events arrive when the client changes the value with a signal, or
/// when the server sets it with `EGUI_STATES_SIGNAL`. With the
/// `EGUI_STATES_PREVIOUS` flag they also carry the replaced value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_subscribe(
    server: *const egui_states_server_t,
    id: u64,
    flags: u32,
) -> egui_states_status_t {
    ffi(|| {
        let previous = has(flags, EGUI_STATES_PREVIOUS, EGUI_STATES_PREVIOUS)?;
        server_ref(server)?.subscribe(id, previous)
    })
}

/// Stops delivering events of a state and drops its pending changes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_unsubscribe(
    server: *const egui_states_server_t,
    id: u64,
) -> egui_states_status_t {
    ffi(|| server_ref(server)?.unsubscribe(id))
}

/// Selects how pending changes of a state are delivered:
/// `EGUI_STATES_MODE_QUEUE` delivers every change in order, and
/// `EGUI_STATES_MODE_SINGLE` coalesces pending changes to the latest one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_set_queue_mode(
    server: *const egui_states_server_t,
    id: u64,
    mode: egui_states_mode_t,
) -> egui_states_status_t {
    ffi(|| {
        let queue = match mode {
            EGUI_STATES_MODE_SINGLE => false,
            EGUI_STATES_MODE_QUEUE => true,
            _ => {
                return Err(Error::new(
                    ErrorCode::InvalidArgument,
                    format!("{mode} is not an EGUI_STATES_MODE_* value"),
                ));
            }
        };
        server_ref(server)?.set_queue_mode(id, queue)
    })
}

/// Waits up to `timeout_ms` milliseconds for the next change of a subscribed
/// state; 0 only checks, and `EGUI_STATES_WAIT_INFINITE` waits without limit.
///
/// Returns `EGUI_STATES_ERROR_TIMEOUT` when nothing arrived and
/// `EGUI_STATES_ERROR_CLOSED` after `egui_states_server_close_events`.
/// Several threads may wait at once; each event goes to one of them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_wait_event(
    server: *const egui_states_server_t,
    timeout_ms: u64,
    out_event: *mut *mut egui_states_event_t,
) -> egui_states_status_t {
    ffi(|| {
        output(out_event, "out_event")?;
        let deadline = Instant::now().checked_add(Duration::from_millis(timeout_ms));
        let deadline = if timeout_ms == u64::MAX {
            None
        } else {
            deadline
        };
        let change = server_ref(server)?.wait_event(deadline)?;
        let handle = Box::into_raw(Box::new(egui_states_event_t(change)));
        unsafe { write(out_event, handle, "out_event") }
    })
}

/// Wakes every thread in `egui_states_server_wait_event` and makes current
/// and future waits return `EGUI_STATES_ERROR_CLOSED`. This cannot be undone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_server_close_events(
    server: *const egui_states_server_t,
) -> egui_states_status_t {
    ffi(|| {
        server_ref(server)?.close_events();
        Ok(())
    })
}

/// Frees an event and releases its claim on the state. Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_event_free(event: *mut egui_states_event_t) {
    unsafe { free(event) }
}

/// Reports the id of the changed state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_event_id(
    event: *const egui_states_event_t,
    out_id: *mut u64,
) -> egui_states_status_t {
    ffi(|| unsafe { write(out_id, event_ref(event)?.id(), "out_id") })
}

/// Loads the new value into `reader`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_event_value(
    event: *const egui_states_event_t,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| {
        event_ref(event)?.load_value(reader_mut(reader)?);
        Ok(())
    })
}

/// Reports whether the event carries the replaced value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_event_has_previous(
    event: *const egui_states_event_t,
    out_has_previous: *mut bool,
) -> egui_states_status_t {
    ffi(|| unsafe {
        write(
            out_has_previous,
            event_ref(event)?.has_previous(),
            "out_has_previous",
        )
    })
}

/// Loads the replaced value into `reader`, or returns
/// `EGUI_STATES_ERROR_NOT_FOUND` when the event has none.
///
/// In single mode, coalesced changes report the value last delivered: for
/// `a -> b -> c` delivered as one event, the previous value is `a`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_event_previous(
    event: *const egui_states_event_t,
    reader: *mut egui_states_reader_t,
) -> egui_states_status_t {
    ffi(|| event_ref(event)?.load_previous(reader_mut(reader)?))
}
