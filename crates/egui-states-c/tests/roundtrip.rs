//! A Rust client synchronizes every state kind with a server driven only
//! through the C functions, registered with hand-written type strings.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use egui_states::{
    Client, ClientBuilder, ConnectionState, Data, DataMulti, DataMultiTake, DataTake, Image,
    ImageMulti, MapState, Queue, Signal, Static, Value, ValueTake, VecState,
};
use egui_states_c::*;

#[egui_states::typed]
#[derive(Clone, Copy, Debug, Default, PartialEq, egui_states::InitialValue)]
enum Mode {
    #[default]
    Idle = 3,
    Busy = 17,
}

#[egui_states::typed]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
struct Point {
    x: i32,
    label: String,
    mode: Mode,
}

#[derive(egui_states::State)]
struct Nested {
    flag: Static<bool>,
}

#[derive(egui_states::State)]
struct AllKinds {
    count: Value<i32>,
    point: Value<Point, Queue>,
    title: Static<String>,
    take: ValueTake<(u8, String)>,
    pulse: Signal<()>,
    pair: Signal<(u16, Option<Mode>), Queue>,
    items: VecState<Option<u32>>,
    lookup: MapState<String, Vec<i16>>,
    image: Image,
    images: ImageMulti,
    samples: Data<f32>,
    once: DataTake<u8>,
    channels: DataMulti<i64>,
    burst: DataMultiTake<u16>,
    nested: Nested,
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(egui_states_last_error()) }
        .to_string_lossy()
        .into_owned()
}

#[track_caller]
fn ok(status: egui_states_status_t) {
    assert_eq!(status, EGUI_STATES_OK, "{}", last_error());
}

fn wait(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out: {label}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct Writer(*mut egui_states_writer_t);

impl Writer {
    fn new(write: impl FnOnce(&Writer)) -> Self {
        let mut handle = null_mut();
        ok(unsafe { egui_states_writer_new(&mut handle) });
        let writer = Self(handle);
        write(&writer);
        writer
    }

    fn i32(&self, value: i32) {
        ok(unsafe { egui_states_write_i32(self.0, value) });
    }

    fn u32(&self, value: u32) {
        ok(unsafe { egui_states_write_u32(self.0, value) });
    }

    fn string(&self, value: &str) {
        ok(unsafe { egui_states_write_string(self.0, value.as_ptr().cast(), value.len()) });
    }

    fn some(&self) {
        ok(unsafe { egui_states_write_some(self.0) });
    }

    fn none(&self) {
        ok(unsafe { egui_states_write_none(self.0) });
    }

    fn len(&self, count: usize) {
        ok(unsafe { egui_states_write_len(self.0, count) });
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
        let mut handle = null_mut();
        ok(unsafe { egui_states_reader_new(&mut handle) });
        Self(handle)
    }

    fn i32(&self) -> i32 {
        let mut value = 0;
        ok(unsafe { egui_states_read_i32(self.0, &mut value) });
        value
    }

    fn u16(&self) -> u16 {
        let mut value = 0;
        ok(unsafe { egui_states_read_u16(self.0, &mut value) });
        value
    }

    fn string(&self) -> String {
        let mut data: *const c_char = null();
        let mut len = 0;
        ok(unsafe { egui_states_read_string(self.0, &mut data, &mut len) });
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn option(&self) -> bool {
        let mut present = false;
        ok(unsafe { egui_states_read_option(self.0, &mut present) });
        present
    }

    fn variant(&self) -> u32 {
        let mut index = 0;
        ok(unsafe { egui_states_read_enum(self.0, &mut index) });
        index
    }

    fn u32(&self) -> u32 {
        let mut value = 0;
        ok(unsafe { egui_states_read_u32(self.0, &mut value) });
        value
    }

    fn at_end(&self) -> bool {
        let mut at_end = false;
        ok(unsafe { egui_states_reader_at_end(self.0, &mut at_end) });
        at_end
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { egui_states_reader_free(self.0) };
    }
}

struct Server {
    handle: *mut egui_states_server_t,
    ids: HashMap<&'static str, u64>,
}

impl Server {
    fn id(&self, name: &str) -> u64 {
        self.ids[name]
    }

    /// Waits for the next event and returns its state id with a reader of
    /// its value and of its previous value, if any.
    fn next_event(&self) -> (u64, Reader, Option<Reader>) {
        let mut event = null_mut();
        ok(unsafe { egui_states_server_wait_event(self.handle, 5000, &mut event) });
        let mut id = 0;
        let value = Reader::new();
        let mut has_previous = false;
        unsafe {
            ok(egui_states_event_id(event, &mut id));
            ok(egui_states_event_value(event, value.0));
            ok(egui_states_event_has_previous(event, &mut has_previous));
        }
        let previous = has_previous.then(|| {
            let previous = Reader::new();
            ok(unsafe { egui_states_event_previous(event, previous.0) });
            previous
        });
        // Freeing the event releases its claim; the readers keep the values.
        unsafe { egui_states_event_free(event) };
        (id, value, previous)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            egui_states_server_close_events(self.handle);
            egui_states_server_free(self.handle);
        }
    }
}

fn register() -> Server {
    let mut handle = null_mut();
    ok(unsafe { egui_states_server_new(null(), &mut handle) });
    let mut server = Server {
        handle,
        ids: HashMap::new(),
    };
    let s = server.handle;
    let mut add = |name: &'static str,
                   register: &dyn Fn(&CStr, *mut u64) -> egui_states_status_t| {
        let full = CString::new(format!("root.{name}")).unwrap();
        let mut id = 0;
        ok(register(&full, &mut id));
        server.ids.insert(name, id);
    };

    unsafe {
        ok(egui_states_server_define_type(
            s,
            c"enum Mode { Idle = 3, Busy = 17 }".as_ptr(),
        ));
        ok(egui_states_server_define_type(
            s,
            c"struct Point { x: i32, label: String, mode: Mode }".as_ptr(),
        ));
    }
    let count = Writer::new(|w| w.i32(5));
    let point = Writer::new(|w| {
        w.i32(1);
        w.string("one");
        ok(unsafe { egui_states_write_enum(w.0, 1) });
    });
    let title = Writer::new(|w| w.string("hello"));
    let flag = Writer::new(|w| ok(unsafe { egui_states_write_bool(w.0, 1) }));
    unsafe {
        add("count", &|name, id| {
            egui_states_server_add_value(s, name.as_ptr(), c"i32".as_ptr(), count.0, 0, id)
        });
        add("point", &|name, id| {
            egui_states_server_add_value(
                s,
                name.as_ptr(),
                c"Point".as_ptr(),
                point.0,
                EGUI_STATES_QUEUE,
                id,
            )
        });
        add("title", &|name, id| {
            egui_states_server_add_static(s, name.as_ptr(), c"String".as_ptr(), title.0, id)
        });
        add("take", &|name, id| {
            egui_states_server_add_value_take(s, name.as_ptr(), c"(u8, String)".as_ptr(), id)
        });
        add("pulse", &|name, id| {
            egui_states_server_add_signal(s, name.as_ptr(), c"()".as_ptr(), 0, id)
        });
        add("pair", &|name, id| {
            egui_states_server_add_signal(
                s,
                name.as_ptr(),
                c"(u16, Option<Mode>)".as_ptr(),
                EGUI_STATES_QUEUE,
                id,
            )
        });
        add("items", &|name, id| {
            egui_states_server_add_vec(s, name.as_ptr(), c"Option<u32>".as_ptr(), id)
        });
        add("lookup", &|name, id| {
            egui_states_server_add_map(
                s,
                name.as_ptr(),
                c"String".as_ptr(),
                c"Vec<i16>".as_ptr(),
                id,
            )
        });
        add("image", &|name, id| {
            egui_states_server_add_image(s, name.as_ptr(), id)
        });
        add("images", &|name, id| {
            egui_states_server_add_image_multi(s, name.as_ptr(), id)
        });
        add("samples", &|name, id| {
            egui_states_server_add_data(
                s,
                name.as_ptr(),
                EGUI_STATES_DATA_F32,
                std::ptr::null(),
                id,
            )
        });
        add("once", &|name, id| {
            egui_states_server_add_data_take(
                s,
                name.as_ptr(),
                EGUI_STATES_DATA_U8,
                std::ptr::null(),
                id,
            )
        });
        add("channels", &|name, id| {
            egui_states_server_add_data_multi(
                s,
                name.as_ptr(),
                EGUI_STATES_DATA_I64,
                std::ptr::null(),
                id,
            )
        });
        add("burst", &|name, id| {
            egui_states_server_add_data_multi_take(
                s,
                name.as_ptr(),
                EGUI_STATES_DATA_U16,
                std::ptr::null(),
                id,
            )
        });
        add("nested.flag", &|name, id| {
            egui_states_server_add_static(s, name.as_ptr(), c"bool".as_ptr(), flag.0, id)
        });
        ok(egui_states_server_finalize(s));
    }
    server
}

fn free_port() -> u16 {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Connection(Client);

impl Drop for Connection {
    fn drop(&mut self) {
        self.0.disconnect();
    }
}

#[test]
fn client_and_c_server_synchronize_every_state_kind() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("the round trip exceeded its hard timeout");
        std::process::exit(124);
    });

    let server = register();
    let s = server.handle;
    let id = |name| server.id(name);

    // Server-side state that the first synchronization delivers.
    let items = Writer::new(|w| {
        w.len(2);
        w.some();
        w.u32(1);
        w.none();
    });
    let lookup = Writer::new(|w| {
        w.len(1);
        w.string("a");
        w.len(2);
        ok(unsafe { egui_states_write_i16(w.0, -1) });
        ok(unsafe { egui_states_write_i16(w.0, 2) });
    });
    let samples = [0.5f32, 1.5];
    let channel = [i64::MIN, 7];
    let gray = [0u8, 50, 100, 150, 200, 250];
    let image = egui_states_image_t {
        data: gray.as_ptr(),
        data_len: gray.len(),
        height: 2,
        width: 3,
        row_stride: 0,
        format: EGUI_STATES_IMAGE_GRAY,
    };
    unsafe {
        ok(egui_states_vec_set(s, id("items"), items.0, 0));
        ok(egui_states_map_set(s, id("lookup"), lookup.0, 0));
        ok(egui_states_data_set(
            s,
            id("samples"),
            EGUI_STATES_DATA_F32,
            samples.as_ptr().cast(),
            2,
            std::ptr::null(),
            0,
        ));
        ok(egui_states_data_multi_set(
            s,
            id("channels"),
            4,
            EGUI_STATES_DATA_I64,
            channel.as_ptr().cast(),
            2,
            std::ptr::null(),
            0,
        ));
        ok(egui_states_image_set(s, id("image"), &image, 0));
        ok(egui_states_image_multi_set_all(
            s,
            id("images"),
            7,
            2,
            2,
            1,
            2,
            3,
            255,
            0,
        ));
        for state in ["count", "point", "pulse", "pair"] {
            let flags = if state == "count" {
                EGUI_STATES_PREVIOUS
            } else {
                0
            };
            ok(egui_states_server_subscribe(s, id(state), flags));
        }
        ok(egui_states_server_subscribe(
            s,
            EGUI_STATES_ID_ON_CONNECT,
            0,
        ));
        ok(egui_states_server_subscribe(
            s,
            EGUI_STATES_ID_ON_DISCONNECT,
            0,
        ));
    }
    let port = free_port();
    ok(unsafe { egui_states_server_start(s, port, c"127.0.0.1".as_ptr(), null()) });

    let (state, client) = ClientBuilder::<AllKinds>::new(egui::Context::default(), port).build();
    let connection = Connection(client);
    wait("client connection", || {
        connection.0.connect();
        connection.0.get_state() == ConnectionState::Connected
    });
    wait("initial synchronization", || state.count.get() == 5);

    let (event, peer, _) = server.next_event();
    assert_eq!(event, EGUI_STATES_ID_ON_CONNECT);
    assert!(peer.string().starts_with("127.0.0.1:"));

    // Server to client.
    assert_eq!(
        state.point.get(),
        Point {
            x: 1,
            label: "one".into(),
            mode: Mode::Busy
        }
    );
    assert_eq!(state.title.get(), "hello");
    assert!(state.nested.flag.get());
    wait("vector", || state.items.get() == [Some(1), None]);
    wait("map", || {
        state.lookup.get() == HashMap::from([("a".into(), vec![-1, 2])])
    });
    wait("data", || state.samples.get() == Some(samples.to_vec()));
    wait("data multi", || {
        state.channels.get(4) == Some(channel.to_vec())
    });
    wait("image", || state.image.get_size() == Some([3, 2]));
    wait("image multi", || state.images.indices() == [7]);

    let take = Writer::new(|w| {
        ok(unsafe { egui_states_write_u8(w.0, 9) });
        w.string("nine");
    });
    let item = Writer::new(|w| {
        w.some();
        w.u32(3);
    });
    let key = Writer::new(|w| w.string("b"));
    let value = Writer::new(|w| {
        w.len(1);
        ok(unsafe { egui_states_write_i16(w.0, 5) });
    });
    let new_title = Writer::new(|w| w.string("bye"));
    let bytes = [1u8, 2, 3];
    let burst = [10u16, 20];
    unsafe {
        ok(egui_states_value_take_set(
            s,
            id("take"),
            take.0,
            EGUI_STATES_UPDATE,
        ));
        ok(egui_states_data_take_set(
            s,
            id("once"),
            EGUI_STATES_DATA_U8,
            bytes.as_ptr().cast(),
            3,
            std::ptr::null(),
            0,
        ));
        ok(egui_states_data_multi_take_set(
            s,
            id("burst"),
            2,
            EGUI_STATES_DATA_U16,
            burst.as_ptr().cast(),
            2,
            std::ptr::null(),
            0,
        ));
        ok(egui_states_vec_append(s, id("items"), item.0, 0));
        ok(egui_states_map_set_item(s, id("lookup"), key.0, value.0, 0));
        ok(egui_states_static_set(
            s,
            id("title"),
            new_title.0,
            EGUI_STATES_UPDATE,
        ));
        ok(egui_states_server_update(s, 0.0));
    }
    let mut taken = None;
    wait("value take", || {
        taken = state.take.take();
        taken.is_some()
    });
    assert_eq!(taken, Some((9, "nine".to_owned())));
    let mut once = None;
    wait("data take", || {
        once = state.once.take();
        once.is_some()
    });
    assert_eq!(once, Some(bytes.to_vec()));
    let mut burst_taken = None;
    wait("data multi take", || {
        burst_taken = state.burst.take(2);
        burst_taken.is_some()
    });
    assert_eq!(burst_taken, Some(burst.to_vec()));
    wait("appended item", || {
        state.items.get() == [Some(1), None, Some(3)]
    });
    wait("map item", || state.lookup.get().get("b") == Some(&vec![5]));
    wait("static", || state.title.get() == "bye");

    // Client to server.
    state.count.set_signal(6);
    let (event, value, previous) = server.next_event();
    assert_eq!(event, id("count"));
    assert_eq!(value.i32(), 6);
    assert!(value.at_end());
    assert_eq!(
        previous
            .expect("count events carry the previous value")
            .i32(),
        5
    );
    let reader = Reader::new();
    ok(unsafe { egui_states_value_get(s, id("count"), reader.0) });
    assert_eq!(reader.i32(), 6);

    state.point.set_signal(Point {
        x: 2,
        label: "two".into(),
        mode: Mode::Idle,
    });
    let (event, value, previous) = server.next_event();
    assert_eq!(event, id("point"));
    assert!(previous.is_none());
    assert_eq!(
        (value.i32(), value.string(), value.variant()),
        (2, "two".into(), 0)
    );

    state.pulse.set(());
    let (event, value, _) = server.next_event();
    assert_eq!(event, id("pulse"));
    assert!(value.at_end());

    state.pair.set((7, Some(Mode::Busy)));
    let (event, value, _) = server.next_event();
    assert_eq!(event, id("pair"));
    assert_eq!(value.u16(), 7);
    assert!(value.option());
    assert_eq!(value.variant(), 1);

    drop(connection);
    let (event, _, _) = server.next_event();
    assert_eq!(event, EGUI_STATES_ID_ON_DISCONNECT);
    ok(unsafe { egui_states_server_stop(s) });
}

#[derive(egui_states::State)]
struct HeaderKinds {
    framed: Data<f32, (u32, String)>,
    keyed: DataMulti<u16, (u32, String)>,
    plain: Data<u8>,
}

/// A header writer of `(u32, String)`.
fn meta(number: u32, label: &str) -> Writer {
    Writer::new(|w| {
        w.u32(number);
        w.string(label);
    })
}

#[test]
fn data_headers_cross_the_c_abi() {
    let mut s = null_mut();
    ok(unsafe { egui_states_server_new(null(), &mut s) });
    let server = Server {
        handle: s,
        ids: HashMap::new(),
    };
    let header_type = c"(u32, String)".as_ptr();
    let (mut framed, mut keyed, mut plain, mut bad) = (0, 0, 0, 0);
    unsafe {
        ok(egui_states_server_add_data(
            s,
            c"root.framed".as_ptr(),
            EGUI_STATES_DATA_F32,
            header_type,
            &mut framed,
        ));
        ok(egui_states_server_add_data_multi(
            s,
            c"root.keyed".as_ptr(),
            EGUI_STATES_DATA_U16,
            header_type,
            &mut keyed,
        ));
        ok(egui_states_server_add_data(
            s,
            c"root.plain".as_ptr(),
            EGUI_STATES_DATA_U8,
            null(),
            &mut plain,
        ));
        assert_ne!(
            egui_states_server_add_data(
                s,
                c"root.bad".as_ptr(),
                EGUI_STATES_DATA_U8,
                c"NoSuchType".as_ptr(),
                &mut bad,
            ),
            EGUI_STATES_OK
        );
        ok(egui_states_server_finalize(s));
    }

    let reader = Reader::new();
    let values = [1.0f32, 2.0];
    let more = [3.0f32];
    unsafe {
        let get_header = |id| egui_states_data_get_header(s, id, reader.0);
        let mut count = 0;
        // The data starts "not set": only a set with a header creates it.
        assert_eq!(get_header(framed), EGUI_STATES_ERROR_NOT_FOUND);
        assert_eq!(
            egui_states_data_len(s, framed, &mut count),
            EGUI_STATES_ERROR_NOT_FOUND
        );
        assert_eq!(
            egui_states_data_set(
                s,
                framed,
                EGUI_STATES_DATA_F32,
                values.as_ptr().cast(),
                2,
                null(),
                0,
            ),
            EGUI_STATES_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            egui_states_data_add(
                s,
                framed,
                EGUI_STATES_DATA_F32,
                more.as_ptr().cast(),
                1,
                null(),
                0,
            ),
            EGUI_STATES_ERROR_NOT_FOUND
        );
        let seven = meta(7, "seven");
        ok(egui_states_data_set(
            s,
            framed,
            EGUI_STATES_DATA_F32,
            values.as_ptr().cast(),
            2,
            seven.0,
            0,
        ));
        // A null header keeps the current one.
        ok(egui_states_data_add(
            s,
            framed,
            EGUI_STATES_DATA_F32,
            more.as_ptr().cast(),
            1,
            null(),
            0,
        ));
        ok(get_header(framed));
        assert_eq!((reader.u32(), reader.string()), (7, "seven".to_string()));
        assert!(reader.at_end());

        // A state without a header type has no header to read or set.
        assert_eq!(get_header(plain), EGUI_STATES_ERROR_NOT_FOUND);
        assert_eq!(
            egui_states_data_set_header(s, plain, seven.0, 0),
            EGUI_STATES_ERROR_INVALID_ARGUMENT
        );
        let large = meta(1, &"x".repeat(1024 * 1024));
        assert_eq!(
            egui_states_data_set_header(s, framed, large.0, 0),
            EGUI_STATES_ERROR_OUT_OF_RANGE
        );

        let keyed_header = |index| egui_states_data_multi_get_header(s, keyed, index, reader.0);
        let nine = meta(9, "nine");
        assert_eq!(keyed_header(1), EGUI_STATES_ERROR_NOT_FOUND);
        assert_eq!(
            egui_states_data_multi_set_header(s, keyed, 1, nine.0, 0),
            EGUI_STATES_ERROR_NOT_FOUND
        );
        // A new index needs a header.
        let five = [5u16];
        let set_five = |header| {
            egui_states_data_multi_set(
                s,
                keyed,
                1,
                EGUI_STATES_DATA_U16,
                five.as_ptr().cast(),
                1,
                header,
                0,
            )
        };
        assert_eq!(set_five(null()), EGUI_STATES_ERROR_INVALID_ARGUMENT);
        assert_eq!(keyed_header(1), EGUI_STATES_ERROR_NOT_FOUND);
        let eight = meta(8, "eight");
        ok(set_five(eight.0));
        ok(keyed_header(1));
        assert_eq!((reader.u32(), reader.string()), (8, "eight".to_string()));
        ok(egui_states_data_multi_set_header(s, keyed, 1, nine.0, 0));
        ok(keyed_header(1));
        assert_eq!((reader.u32(), reader.string()), (9, "nine".to_string()));

        // Without a header type, a set creates the data alone, and a reset
        // returns it to "not set".
        let one = [1u8];
        let set_plain = || {
            egui_states_data_set(
                s,
                plain,
                EGUI_STATES_DATA_U8,
                one.as_ptr().cast(),
                1,
                null(),
                0,
            )
        };
        ok(set_plain());
        ok(egui_states_data_len(s, plain, &mut count));
        assert_eq!(count, 1);
        ok(egui_states_data_reset(s, plain, 0));
        assert_eq!(
            egui_states_data_len(s, plain, &mut count),
            EGUI_STATES_ERROR_NOT_FOUND
        );
        assert_eq!(
            egui_states_data_clear(s, plain, null(), 0),
            EGUI_STATES_ERROR_NOT_FOUND
        );
        ok(egui_states_data_reset(s, plain, 0));
    }

    // A client gets the stored headers with their buffers.
    let port = free_port();
    ok(unsafe { egui_states_server_start(s, port, c"127.0.0.1".as_ptr(), null()) });
    let (state, client) = ClientBuilder::<HeaderKinds>::new(egui::Context::default(), port).build();
    let connection = Connection(client);
    wait("client connection", || {
        connection.0.connect();
        connection.0.get_state() == ConnectionState::Connected
    });
    wait("headers", || {
        state.framed.header() == Some((7, "seven".to_string()))
            && state.keyed.header(1) == Some((9, "nine".to_string()))
    });
    state.framed.read_with_header(|entry| {
        let (values, header) = entry.expect("framed data is set");
        assert_eq!(values, [1.0, 2.0, 3.0]);
        assert_eq!(header.0, 7);
    });
    assert_eq!(state.keyed.get(1), Some(vec![5]));
    assert_eq!(state.plain.get(), None);
    drop(connection);
    unsafe { egui_states_server_stop(s) };
    drop(server);
}
