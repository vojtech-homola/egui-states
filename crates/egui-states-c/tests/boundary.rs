//! Argument checking, status codes, and error reporting at the C boundary.

use std::ffi::{CStr, c_char, c_void};
use std::ptr::{null, null_mut};

use egui_states_c::*;

fn last_error() -> String {
    unsafe { CStr::from_ptr(egui_states_last_error()) }
        .to_string_lossy()
        .into_owned()
}

#[track_caller]
fn ok(status: egui_states_status_t) {
    assert_eq!(status, EGUI_STATES_OK, "{}", last_error());
}

#[track_caller]
fn fails(status: egui_states_status_t, expected: egui_states_status_t, message: &str) {
    assert_eq!(status, expected, "{}", last_error());
    let error = last_error();
    assert!(
        error.contains(message),
        "{error:?} does not contain {message:?}"
    );
}

struct Server(*mut egui_states_server_t);

impl Server {
    fn new() -> Self {
        let mut server = null_mut();
        ok(unsafe { egui_states_server_new(null(), &mut server) });
        Self(server)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe { egui_states_server_free(self.0) };
    }
}

struct Writer(*mut egui_states_writer_t);

impl Writer {
    fn new() -> Self {
        let mut writer = null_mut();
        ok(unsafe { egui_states_writer_new(&mut writer) });
        Self(writer)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        unsafe { egui_states_writer_free(self.0) };
    }
}

struct Reader(*mut egui_states_reader_t);

impl Reader {
    fn new() -> Self {
        let mut reader = null_mut();
        ok(unsafe { egui_states_reader_new(&mut reader) });
        Self(reader)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { egui_states_reader_free(self.0) };
    }
}

/// A finalized server with one state of several kinds.
struct Fixture {
    server: Server,
    value: u64,
    image: u64,
    images: u64,
    data: u64,
    take: u64,
}

fn fixture() -> Fixture {
    let server = Server::new();
    let initial = Writer::new();
    let mut ids = [0u64; 5];
    unsafe {
        ok(egui_states_write_i32(initial.0, 1));
        ok(egui_states_server_add_value(
            server.0,
            c"root.value".as_ptr(),
            c"i32".as_ptr(),
            initial.0,
            0,
            &mut ids[0],
        ));
        ok(egui_states_server_add_image(
            server.0,
            c"root.image".as_ptr(),
            &mut ids[1],
        ));
        ok(egui_states_server_add_image_multi(
            server.0,
            c"root.images".as_ptr(),
            &mut ids[2],
        ));
        ok(egui_states_server_add_data(
            server.0,
            c"root.data".as_ptr(),
            EGUI_STATES_DATA_U16,
            &mut ids[3],
        ));
        ok(egui_states_server_add_data_take(
            server.0,
            c"root.take".as_ptr(),
            EGUI_STATES_DATA_U8,
            &mut ids[4],
        ));
        ok(egui_states_server_finalize(server.0));
    }
    Fixture {
        server,
        value: ids[0],
        image: ids[1],
        images: ids[2],
        data: ids[3],
        take: ids[4],
    }
}

#[test]
fn meta_functions_report_versions_and_ids() {
    assert_eq!(egui_states_abi_version(), EGUI_STATES_ABI_VERSION);
    assert!(egui_states_protocol_version() > 0);
    let version = unsafe { CStr::from_ptr(egui_states_library_version()) };
    assert_eq!(version.to_str().unwrap(), env!("CARGO_PKG_VERSION"));

    let mut id = 0;
    ok(unsafe { egui_states_state_id(c"root.value".as_ptr(), &mut id) });
    assert_eq!(id, egui_states::c_abi::state_id("root.value"));
    assert!(id > 9);
}

#[test]
fn null_pointers_are_rejected() {
    unsafe {
        fails(
            egui_states_server_new(null(), null_mut()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`out_server` must not be null",
        );
        fails(
            egui_states_writer_new(null_mut()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`out_writer`",
        );
        fails(
            egui_states_write_i32(null_mut(), 1),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`writer` must not be null",
        );
        fails(
            egui_states_server_finalize(null()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`server` must not be null",
        );
        let f = fixture();
        fails(
            egui_states_value_get(f.server.0, f.value, null_mut()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`reader`",
        );
        fails(
            egui_states_value_set(f.server.0, f.value, null(), 0),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`value`",
        );
        fails(
            egui_states_server_add_image(f.server.0, null(), null_mut()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`name`",
        );
        fails(
            egui_states_vec_len(f.server.0, f.value, null_mut()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`out_len`",
        );

        // Freeing null is a no-op.
        egui_states_server_free(null_mut());
        egui_states_writer_free(null_mut());
        egui_states_reader_free(null_mut());
        egui_states_event_free(null_mut());
    }
}

#[test]
fn flags_and_enum_values_are_validated() {
    let f = fixture();
    let writer = Writer::new();
    let server = f.server.0;
    unsafe {
        ok(egui_states_write_i32(writer.0, 2));
        fails(
            egui_states_value_set(server, f.value, writer.0, EGUI_STATES_CACHE),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "unsupported flag bits 0x8",
        );
        ok(egui_states_value_set(
            server,
            f.value,
            writer.0,
            EGUI_STATES_UPDATE | EGUI_STATES_SIGNAL,
        ));
        fails(
            egui_states_data_take_set(
                server,
                f.take,
                EGUI_STATES_DATA_U8,
                null(),
                0,
                EGUI_STATES_FORCE,
            ),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "unsupported flag bits",
        );
        fails(
            egui_states_server_subscribe(server, f.value, EGUI_STATES_UPDATE),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "unsupported flag bits",
        );
        fails(
            egui_states_server_set_queue_mode(server, f.value, 2),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "EGUI_STATES_MODE_",
        );
        fails(
            egui_states_data_set(server, f.data, 10, null(), 0, 0),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "10 is not an EGUI_STATES_DATA_* value",
        );
        fails(
            egui_states_write_bool(writer.0, 2),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "must be 0 or 1",
        );
        let pixel = [0u8; 4];
        let image = egui_states_image_t {
            data: pixel.as_ptr(),
            data_len: 4,
            height: 1,
            width: 1,
            row_stride: 0,
            format: 4,
        };
        fails(
            egui_states_image_set(server, f.image, &image, 0),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "EGUI_STATES_IMAGE_",
        );
        fails(
            egui_states_data_set(server, f.data, EGUI_STATES_DATA_U32, null(), 0, 0),
            EGUI_STATES_ERROR_TYPE_MISMATCH,
            "holds u16 elements, not u32",
        );
    }
}

#[test]
fn strings_are_utf8_with_explicit_or_nul_terminated_length() {
    let server = Server::new();
    let writer = Writer::new();
    let reader = Reader::new();
    unsafe {
        fails(
            egui_states_server_add_image(server.0, c"root.\xff".as_ptr(), null_mut()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`name` is not UTF-8",
        );
        fails(
            egui_states_write_string(writer.0, c"\xfe".as_ptr(), 1),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "not UTF-8",
        );
        // Explicit lengths may include NUL; a NUL-terminated length stops at it.
        ok(egui_states_write_string(
            writer.0,
            b"a\0b".as_ptr().cast(),
            3,
        ));
        ok(egui_states_write_string(
            writer.0,
            c"tail".as_ptr(),
            EGUI_STATES_NUL_TERMINATED,
        ));
        ok(egui_states_write_string(writer.0, null(), 0));
        let mut id = 0;
        ok(egui_states_server_add_static(
            server.0,
            c"root.text".as_ptr(),
            c"(String, String, String)".as_ptr(),
            writer.0,
            &mut id,
        ));
        ok(egui_states_server_finalize(server.0));
        ok(egui_states_static_get(server.0, id, reader.0));
        for expected in [&b"a\0b"[..], b"tail", b""] {
            let mut data: *const c_char = null();
            let mut len = 0;
            ok(egui_states_read_string(reader.0, &mut data, &mut len));
            let text = std::slice::from_raw_parts(data.cast::<u8>(), len);
            assert_eq!(text, expected);
        }
    }
}

#[test]
fn copy_out_reports_sizes_and_short_buffers() {
    let f = fixture();
    let server = f.server.0;
    unsafe {
        // Names: a sizing query, a short buffer, then a copy with its NUL.
        let mut len = 0;
        ok(egui_states_server_state_name(
            server,
            f.value,
            null_mut(),
            0,
            &mut len,
        ));
        assert_eq!(len, "root.value".len());
        let mut buffer = [0 as c_char; 16];
        fails(
            egui_states_server_state_name(server, f.value, buffer.as_mut_ptr(), len, &mut len),
            EGUI_STATES_ERROR_BUFFER_TOO_SMALL,
            "11 are needed",
        );
        assert_eq!(len, 10);
        ok(egui_states_server_state_name(
            server,
            f.value,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut len,
        ));
        assert_eq!(
            CStr::from_ptr(buffer.as_ptr()).to_str().unwrap(),
            "root.value"
        );
        fails(
            egui_states_server_state_name(server, f.value, null_mut(), 4, &mut len),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "`buffer`",
        );
        ok(egui_states_server_state_type(
            server,
            f.value,
            buffer.as_mut_ptr(),
            16,
            &mut len,
        ));
        assert_eq!(CStr::from_ptr(buffer.as_ptr()).to_str().unwrap(), "i32");
        fails(
            egui_states_server_state_type(server, f.image, buffer.as_mut_ptr(), 16, &mut len),
            EGUI_STATES_ERROR_NOT_FOUND,
            "has no value type",
        );

        // Image pixels.
        ok(egui_states_image_set_all(
            server, f.image, 2, 3, 1, 2, 3, 4, 0,
        ));
        let (mut required, mut height, mut width) = (0, 0, 0);
        ok(egui_states_image_get(
            server,
            f.image,
            null_mut(),
            0,
            &mut required,
            &mut height,
            &mut width,
        ));
        assert_eq!((required, height, width), (24, 2, 3));
        let mut pixels = vec![0u8; 24];
        fails(
            egui_states_image_get(
                server,
                f.image,
                pixels.as_mut_ptr(),
                23,
                &mut required,
                null_mut(),
                null_mut(),
            ),
            EGUI_STATES_ERROR_BUFFER_TOO_SMALL,
            "24 are needed",
        );
        ok(egui_states_image_get(
            server,
            f.image,
            pixels.as_mut_ptr(),
            24,
            &mut required,
            null_mut(),
            null_mut(),
        ));
        assert_eq!(&pixels[..4], &[1, 2, 3, 4]);

        // Numbers, counted in elements.
        let values = [7u16, 8, 9];
        ok(egui_states_data_set(
            server,
            f.data,
            EGUI_STATES_DATA_U16,
            values.as_ptr().cast(),
            3,
            0,
        ));
        let mut count = 0;
        ok(egui_states_data_get(
            server,
            f.data,
            EGUI_STATES_DATA_U16,
            null_mut(),
            0,
            &mut count,
        ));
        assert_eq!(count, 3);
        let mut out = [0u16; 3];
        fails(
            egui_states_data_get(
                server,
                f.data,
                EGUI_STATES_DATA_U16,
                out.as_mut_ptr().cast(),
                2,
                &mut count,
            ),
            EGUI_STATES_ERROR_BUFFER_TOO_SMALL,
            "3 are needed",
        );
        ok(egui_states_data_get(
            server,
            f.data,
            EGUI_STATES_DATA_U16,
            out.as_mut_ptr().cast(),
            3,
            &mut count,
        ));
        assert_eq!(out, values);
        // An empty buffer may come from a null pointer.
        ok(egui_states_data_add(
            server,
            f.data,
            EGUI_STATES_DATA_U16,
            null(),
            0,
            0,
        ));
        ok(egui_states_data_len(server, f.data, &mut count));
        assert_eq!(count, 3);

        // Image indices.
        ok(egui_states_image_multi_set_all(
            server, f.images, 5, 1, 1, 0, 0, 0, 255, 0,
        ));
        ok(egui_states_image_multi_set_all(
            server, f.images, 9, 1, 1, 0, 0, 0, 255, 0,
        ));
        ok(egui_states_image_multi_indices(
            server,
            f.images,
            null_mut(),
            0,
            &mut count,
        ));
        assert_eq!(count, 2);
        let mut indices = [0u32; 2];
        fails(
            egui_states_image_multi_indices(server, f.images, indices.as_mut_ptr(), 1, &mut count),
            EGUI_STATES_ERROR_BUFFER_TOO_SMALL,
            "2 are needed",
        );
        ok(egui_states_image_multi_indices(
            server,
            f.images,
            indices.as_mut_ptr(),
            2,
            &mut count,
        ));
        indices.sort_unstable();
        assert_eq!(indices, [5, 9]);
    }
}

#[test]
fn readers_and_writers_follow_the_value_type() {
    let server = Server::new();
    let writer = Writer::new();
    let reader = Reader::new();
    unsafe {
        ok(egui_states_server_define_type(
            server.0,
            c"struct Sample { id: u16, scores: Vec<f32>, label: Option<String> }".as_ptr(),
        ));
        let mut hash = 0;
        ok(egui_states_server_type_hash(
            server.0,
            c"Vec<Sample>".as_ptr(),
            &mut hash,
        ));
        fails(
            egui_states_server_type_hash(server.0, c"Vec<Sample".as_ptr(), &mut hash),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "expected `>` at offset 10",
        );

        ok(egui_states_write_u16(writer.0, 3));
        let scores = [0.5f32, -1.0];
        ok(egui_states_write_len(writer.0, 2));
        ok(egui_states_write_array(
            writer.0,
            EGUI_STATES_DATA_F32,
            scores.as_ptr().cast(),
            2,
        ));
        ok(egui_states_write_none(writer.0));
        let mut id = 0;
        ok(egui_states_server_add_static(
            server.0,
            c"root.sample".as_ptr(),
            c"Sample".as_ptr(),
            writer.0,
            &mut id,
        ));
        fails(
            egui_states_server_add_static(
                server.0,
                c"root.wrong".as_ptr(),
                c"(u16, Vec<f64>, Option<String>)".as_ptr(),
                writer.0,
                null_mut(),
            ),
            EGUI_STATES_ERROR_TYPE_MISMATCH,
            "does not match f64",
        );
        ok(egui_states_server_finalize(server.0));
        fails(
            egui_states_server_define_type(server.0, c"struct Late {}".as_ptr()),
            EGUI_STATES_ERROR_INVALID_STATE,
            "after the server is finalized",
        );

        ok(egui_states_static_get(server.0, id, reader.0));
        let mut kind = 0;
        ok(egui_states_reader_peek(reader.0, &mut kind));
        assert_eq!(kind, EGUI_STATES_KIND_U16);
        let mut number = 0u32;
        fails(
            egui_states_read_u32(reader.0, &mut number),
            EGUI_STATES_ERROR_TYPE_MISMATCH,
            "cannot read u32 at .id: the value is u16",
        );
        let mut id_value = 0u16;
        ok(egui_states_read_u16(reader.0, &mut id_value));
        assert_eq!(id_value, 3);
        let mut count = 0;
        ok(egui_states_read_len(reader.0, &mut count));
        let mut read_scores = [0f32; 2];
        ok(egui_states_read_array(
            reader.0,
            EGUI_STATES_DATA_F32,
            read_scores.as_mut_ptr().cast::<c_void>(),
            count,
        ));
        assert_eq!(read_scores, scores);
        let mut present = true;
        ok(egui_states_read_option(reader.0, &mut present));
        assert!(!present);
        let mut at_end = false;
        ok(egui_states_reader_at_end(reader.0, &mut at_end));
        assert!(at_end);
        ok(egui_states_reader_peek(reader.0, &mut kind));
        assert_eq!(kind, EGUI_STATES_KIND_END);
    }
}

#[test]
fn last_error_is_per_thread_and_survives_success() {
    let server = Server::new();
    unsafe {
        fails(
            egui_states_server_add_value(
                server.0,
                c"".as_ptr(),
                c"()".as_ptr(),
                null(),
                0,
                null_mut(),
            ),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "state names cannot be empty",
        );
        ok(egui_states_server_add_value(
            server.0,
            c"root.unit".as_ptr(),
            c"()".as_ptr(),
            null(),
            0,
            null_mut(),
        ));
    }
    assert_eq!(last_error(), "state names cannot be empty");
    let other = std::thread::spawn(last_error).join().unwrap();
    assert_eq!(other, "");
}

#[test]
fn lifecycle_errors_and_event_waits() {
    let server = Server::new();
    unsafe {
        let mut event = null_mut();
        fails(
            egui_states_server_wait_event(server.0, 0, &mut event),
            EGUI_STATES_ERROR_INVALID_STATE,
            "not finalized",
        );
        fails(
            egui_states_server_start(server.0, 0, c"127.0.0.1".as_ptr(), null()),
            EGUI_STATES_ERROR_INVALID_STATE,
            "not finalized",
        );
        ok(egui_states_server_finalize(server.0));
        ok(egui_states_server_finalize(server.0));
        fails(
            egui_states_server_start(server.0, 0, c"localhost".as_ptr(), null()),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "not an IPv4 address",
        );
        fails(
            egui_states_server_update(server.0, -1.0),
            EGUI_STATES_ERROR_INVALID_ARGUMENT,
            "non-negative",
        );
        let mut running = true;
        ok(egui_states_server_is_running(server.0, &mut running));
        assert!(!running);

        ok(egui_states_server_subscribe(
            server.0,
            EGUI_STATES_ID_ON_CONNECT,
            0,
        ));
        fails(
            egui_states_server_wait_event(server.0, 0, &mut event),
            EGUI_STATES_ERROR_TIMEOUT,
            "no event",
        );
        assert!(event.is_null());
        ok(egui_states_server_close_events(server.0));
        fails(
            egui_states_server_wait_event(server.0, EGUI_STATES_WAIT_INFINITE, &mut event),
            EGUI_STATES_ERROR_CLOSED,
            "closed",
        );
        let mut kind = 0;
        ok(egui_states_server_state_kind(
            server.0,
            EGUI_STATES_ID_LOGGING,
            &mut kind,
        ));
        assert_eq!(kind, EGUI_STATES_STATE_SYSTEM);
        fails(
            egui_states_server_state_kind(server.0, 12345, &mut kind),
            EGUI_STATES_ERROR_NOT_FOUND,
            "no state has id 12345",
        );
    }
}
