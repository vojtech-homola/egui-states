//! Deterministic server setup for the integration-only fixture schema.
use crate::bindings::enums::{TestEnum, TestEnum2};
use crate::bindings::structs::{TestStruct, TestStruct2};
use crate::bindings::{State, StatesServer};
use egui_states::server::{self as s, CallbackHandle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Fixture {
    pub server: StatesServer,
    callbacks: Vec<CallbackHandle>,
    errors: Arc<Mutex<Vec<String>>>,
    warnings: Arc<Mutex<Vec<String>>>,
}

type Signals = Arc<Mutex<Vec<String>>>;

/// Command 30: non-default values of every shape for the client to check.
fn set_wire_values(all: &State) -> s::Result<()> {
    let v = &all.values;
    v.bool_value.set(true, false)?;
    v.count.set(-123_456, false)?;
    v.ratio.set(0.375, false)?;
    v.queued_progress.set(0.625, false)?;
    v.title.set("ünïcødé 🦀".into(), false)?;
    v.optional_value.set(Some(-5), false)?;
    v.fixed_numbers.set([1, 65535, 0], false)?;
    v.test_enum.set(TestEnum::C, false)?;
    v.nested.secondary_choice.set(TestEnum2::X, false)?;
    v.nested.selected_enum.set(Some(TestEnum::B), false)?;
    let c = &all.custom_values;
    c.point.set(
        TestStruct {
            x: -1.5,
            y: 2.25,
            label: "point".into(),
        },
        false,
    )?;
    c.optional_struct.set(
        Some(TestStruct2 {
            enabled: true,
            level: 7,
            name: "seven".into(),
        }),
        false,
    )?;
    let st = &all.statics;
    st.status_text.set("ready".into(), false)?;
    st.summary.set(
        TestStruct2 {
            enabled: false,
            level: 65535,
            name: "max".into(),
        },
        false,
    )?;
    st.pair.set([0.5, -0.25], false)?;
    st.nested.label.set("label".into(), false)?;
    st.nested.enum_hint.set(TestEnum::C, false)?;
    let w = &all.wire_types;
    w.small_unsigned.set(255, false)?;
    w.small_signed.set(-128, false)?;
    w.short.set(-32768, false)?;
    w.wide.set(u64::MAX, false)?;
    w.single.set(-0.125, false)?;
    w.tuple.set((-7, "tuple".into(), true), false)?;
    w.list.set(
        vec![
            TestStruct2 {
                enabled: true,
                level: 1,
                name: "a".into(),
            },
            TestStruct2 {
                enabled: false,
                level: 2,
                name: "b".into(),
            },
        ],
        false,
    )?;
    w.table.set(
        HashMap::from([("pi".into(), 3.5), ("neg".into(), -0.5)]),
        false,
    )?;
    w.keyed_enum
        .set(HashMap::from([(TestEnum::A, 1), (TestEnum::C, 3)]), false)?;
    w.optional_enum.set(Some(TestEnum2::Z), false)?;
    w.bytes.set(vec![0, 127, 255], false)
}

/// Command 31: the values and signals the client sent after command 30.
fn check_client_values(all: &State, signals: &Signals) -> Result<(), String> {
    macro_rules! expect {
        ($handle:expr, $expected:expr) => {{
            let actual = $handle.get().map_err(|e| e.to_string())?;
            let expected = $expected;
            if actual != expected {
                return Err(format!(
                    "{}: received {actual:?}, expected {expected:?}",
                    stringify!($handle)
                ));
            }
        }};
    }
    let (v, c, w) = (&all.values, &all.custom_values, &all.wire_types);
    expect!(v.bool_value, false);
    expect!(v.count, 654_321);
    expect!(v.ratio, -0.75);
    expect!(v.queued_progress, 0.125);
    expect!(v.title, "client ✓");
    expect!(v.optional_value, None);
    expect!(v.fixed_numbers, [9, 8, 7]);
    expect!(v.test_enum, TestEnum::A);
    expect!(v.nested.secondary_choice, TestEnum2::Y);
    expect!(v.nested.selected_enum, None);
    expect!(
        c.point,
        TestStruct {
            x: 0.5,
            y: -0.5,
            label: "client".into(),
        }
    );
    expect!(c.optional_struct, None);
    expect!(w.small_unsigned, 1);
    expect!(w.small_signed, 127);
    expect!(w.short, 32767);
    expect!(w.wide, 0);
    expect!(w.single, 0.5);
    expect!(w.tuple, (0, String::new(), false));
    expect!(w.list, Vec::<TestStruct2>::new());
    expect!(w.table, HashMap::<String, f64>::new());
    expect!(w.keyed_enum, HashMap::from([(TestEnum::B, 2)]));
    expect!(w.optional_enum, None);

    // Signals run on other workers; give them time to arrive.
    let expected = ["empty", "enum:C", "number:-2.5", "pair:1.5:B"];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut received = signals.lock().unwrap().clone();
        received.sort();
        if received == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "signals: received {received:?}, expected {expected:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Command 32: server edits of both collections.
fn edit_collections(all: &State) -> s::Result<()> {
    let items = &all.value_vec.items;
    items.set(vec![5, 6, 7], true)?;
    items.set_item(1, 60, true)?;
    items.add_item(8, true)?;
    items.remove_item(0, true)?;
    let map = &all.value_map.items;
    map.set(HashMap::from([(1, 10), (2, 20)]), true)?;
    map.set_item(3, 30, true)?;
    map.set_item(1, 11, true)?;
    map.remove_item(&2, true)?;
    Ok(())
}

/// Command 33: numeric edits and the limits of every element type.
fn edit_numeric(all: &State) -> s::Result<()> {
    let d = &all.data;
    d.bytes.set(&[1, 2, 3, 4, 5], false)?;
    d.bytes.add(&[6], false)?;
    d.bytes.replace(&[20, 30], 1, false)?;
    d.bytes.remove(0, 1, false)?;
    d.samples.set(&[-1.5, 0.25], false)?;
    d.nested.buffer.set(&[1, 2, 3], false)?;
    d.nested.buffer.clear(false)?;
    let n = &all.numeric;
    n.u8_data.set(&[u8::MIN, 0, u8::MAX], false)?;
    n.i8_data.set(&[i8::MIN, 0, i8::MAX], false)?;
    n.u16_data.set(&[u16::MIN, 0, u16::MAX], false)?;
    n.i16_data.set(&[i16::MIN, 0, i16::MAX], false)?;
    n.u32_data.set(&[u32::MIN, 0, u32::MAX], false)?;
    n.i32_data.set(&[i32::MIN, 0, i32::MAX], false)?;
    n.u64_data.set(&[u64::MIN, 0, u64::MAX], false)?;
    n.i64_data.set(&[i64::MIN, 0, i64::MAX], false)?;
    n.f32_data.set(&[f32::MIN, 0.0, f32::MAX], false)?;
    n.f64_data.set(&[f64::MIN, 0.0, f64::MAX], false)?;
    let m = &all.multi_data;
    m.bytes.set(1, &[1, 2], false)?;
    m.bytes.add(1, &[3], false)?;
    m.bytes.set(2, &[9], false)?;
    m.bytes.remove_index(2, false)?;
    m.samples.set(7, &[0.5], false)?;
    m.nested.buffer.set(3, &[4, 5], false)?;
    m.nested.buffer.replace(3, 1, &[6], false)
}

/// Command 34: a full image, a partial update, and sparse images.
fn edit_images(all: &State) -> s::Result<()> {
    let pixels: Vec<u8> = (0..18).collect();
    let image = &all.image.image;
    image.set(&pixels, [2, 3], s::ImageFormat::Color, false)?;
    image.update(
        &[200, 201, 202, 210, 211, 212],
        [1, 1],
        [1, 2],
        s::ImageFormat::Color,
        false,
        false,
    )?;
    let images = &all.image.images;
    images.set(2, &[9], [1, 1], s::ImageFormat::Gray, false)?;
    images.set_all(7, [2, 2], s::ImageColor::Color(1, 2, 3), false)?;
    images.remove_index(2, false)
}

impl Fixture {
    pub fn new(port: u16) -> s::Result<Self> {
        let errors = Arc::new(Mutex::new(Vec::new()));
        let on_error = errors.clone();
        let server = StatesServer::with_options(s::ServerOptions {
            version: Some(StatesServer::VERSION_HASH),
            error_handler: Some(Arc::new(move |error| {
                on_error.lock().unwrap().push(error.to_string())
            })),
            ..s::ServerOptions::default()
        })?;
        let states = &server.states.integration;
        states.value.set(7, false)?;
        states.items.set(vec![1, 2], false)?;
        states.cached.set(&[8, 9], false, false, true)?;
        states
            .cached_multi
            .set(91, &[500, 600], false, false, true)?;
        states.wire.set(
            crate::bindings::structs::WirePacket {
                signed: i64::MIN,
                unsigned: u64::MAX,
                text: "žluťoučký 🦀".into(),
                optional: Some(i32::MIN),
                fixed: [0, 32768, u16::MAX],
                nested: crate::bindings::structs::TestStruct2 {
                    enabled: true,
                    level: u16::MAX,
                    name: "中".into(),
                },
                choice: crate::bindings::enums::TestEnum2::Z,
            },
            false,
        )?;
        let on_error = errors.clone();
        let mut callbacks = vec![server.logging.add_logger(s::LogLevel::Error, move |error| {
            on_error.lock().unwrap().push(error.to_string())
        })];
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let on_warning = warnings.clone();
        callbacks.push(
            server
                .logging
                .add_logger(s::LogLevel::Warning, move |warning| {
                    on_warning.lock().unwrap().push(warning)
                }),
        );
        let signals: Signals = Arc::new(Mutex::new(Vec::new()));
        let record = signals.clone();
        callbacks.push(
            server
                .states
                .signals
                .empty_signal
                .connect_empty(move || record.lock().unwrap().push("empty".into())),
        );
        let record = signals.clone();
        callbacks.push(
            server
                .states
                .signals
                .number_signal
                .connect(move |value| record.lock().unwrap().push(format!("number:{value}"))),
        );
        let record = signals.clone();
        callbacks.push(
            server
                .states
                .signals
                .enum_signal
                .connect(move |value| record.lock().unwrap().push(format!("enum:{value:?}"))),
        );
        let record = signals.clone();
        callbacks.push(
            server
                .states
                .wire_types
                .pair_signal
                .connect(move |(number, choice)| {
                    record
                        .lock()
                        .unwrap()
                        .push(format!("pair:{number}:{choice:?}"))
                }),
        );
        let callback_value = states.callback_value.clone();
        let on_error = errors.clone();
        callbacks.push(states.value.connect(move |value| {
            if let Err(e) = callback_value.set(value, true) {
                on_error.lock().unwrap().push(e.to_string());
            }
        }));
        let echo = states.wire_echo.clone();
        callbacks.push(states.wire.connect(move |packet| {
            echo.set(packet, true).unwrap();
        }));
        let extra_takes = server.states.data_take.clone();
        let extra_multi = server.states.data_multi_take.clone();
        let st = states.clone();
        let all = server.states.clone();
        let on_error = errors.clone();
        callbacks.push(states.command.connect(move |command| {
            let result = (|| -> s::Result<()> {
                match command {
                    1 => st.value.set(21, true)?,
                    2 => {
                        st.items.add_item(st.value.get()?, true)?;
                        st.map.set_item(9, 900, true)?;
                    }
                    3 => {
                        extra_takes
                            .take_samples
                            .set(&[-1.25, 0.0, 2.5], false, true, false)?;
                        extra_multi
                            .samples
                            .set(513, &[9.5, -2.25], false, true, false)?;
                        extra_multi
                            .nested
                            .buffer
                            .set(24, &[0, u16::MAX], false, true, false)?;
                        st.take.set("payload".into(), false, true)?;
                        st.empty.set((), false, true)?;
                        st.data.set(&[0, 1, 255], false, true, false)?;
                        st.multi.set(7, &[100, 200], false, true, false)?;
                        st.multi.set(42, &[], false, true, false)?;
                    }
                    4 => {
                        st.take.set(String::new(), false, true)?;
                        st.data.set(&[], false, true, false)?;
                    }
                    10..=13 => {
                        match command {
                            10 => st.take.set("first".into(), true, true)?,
                            11 => st.empty.set((), true, true)?,
                            12 => st.data.set(&[1, 2], true, true, false)?,
                            _ => st.multi.set(73, &[100, 200], true, true, false)?,
                        }
                        st.phase.set(1, true)?;
                        match command {
                            10 => st.take.set(String::new(), true, true)?,
                            11 => st.empty.set((), true, true)?,
                            12 => st.data.set(&[], true, true, false)?,
                            _ => st.multi.set(73, &[], true, true, false)?,
                        }
                        st.phase.set(2, true)?;
                    }
                    20 => {
                        st.cached.set(&[31, 32], false, true, true)?;
                        st.cached_multi.remove_index(91, true)?;
                        st.cached_multi.set(513, &[17], false, true, true)?;
                        st.phase.set(20, true)?;
                    }
                    21 => {
                        st.cached.set(&[41], false, true, false)?;
                        st.cached_multi.reset(true)?;
                        st.phase.set(21, true)?;
                    }
                    22 => {
                        st.multi.set(7, &[7], false, true, false)?;
                        st.multi.set(900, &[9], false, true, false)?;
                        st.multi.remove_index(7, true)?;
                        st.phase.set(22, true)?;
                    }
                    23 => {
                        st.multi.reset(true)?;
                        st.phase.set(23, true)?;
                    }
                    24 | 25 => {
                        st.multi.set(7, &[7], true, true, false)?;
                        if command == 24 {
                            st.multi.remove_index(7, true)?;
                        } else {
                            st.multi.reset(true)?;
                        }
                        st.phase.set(24, true)?;
                        st.multi.set(513, &[9], true, true, false)?;
                        st.phase.set(25, true)?;
                    }
                    30 => {
                        set_wire_values(&all)?;
                        st.phase.set(30, true)?;
                    }
                    31 => {
                        if let Err(error) = check_client_values(&all, &signals) {
                            on_error.lock().unwrap().push(error);
                        }
                        st.phase.set(31, true)?;
                    }
                    32 => {
                        edit_collections(&all)?;
                        st.phase.set(32, true)?;
                    }
                    33 => {
                        edit_numeric(&all)?;
                        st.phase.set(33, true)?;
                    }
                    34 => {
                        edit_images(&all)?;
                        st.phase.set(34, true)?;
                    }
                    99 => st.phase.set(99, true)?,
                    _ => panic!("unknown fixture command"),
                }
                Ok(())
            })();
            if let Err(error) = result {
                on_error.lock().unwrap().push(error.to_string());
            }
        }));
        server.start(port, Some(std::net::Ipv4Addr::LOCALHOST), None)?;
        Ok(Self {
            server,
            callbacks,
            errors,
            warnings,
        })
    }

    pub fn assert_no_errors(&self) {
        assert!(self.errors.lock().unwrap().is_empty(), "{:?}", self.errors);
    }

    /// Waits for a logged warning containing `text`.
    pub fn assert_warning(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self
            .warnings
            .lock()
            .unwrap()
            .iter()
            .any(|w| w.contains(text))
        {
            assert!(
                Instant::now() < deadline,
                "no warning containing {text:?}: {:?}",
                self.warnings
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.stop();
        self.callbacks.clear();
    }
}
