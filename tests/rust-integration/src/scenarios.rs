//! Identical client assertions for the generated Python and Rust servers.
use egui_states::{Client, ClientBuilder, ConnectionState};
use egui_states_test_schema::{State, integration::IntegrationState};
use std::time::{Duration, Instant};

pub use crate::scenario_registry::SCENARIOS;

pub fn wait<T>(label: &str, mut condition: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = condition() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out: {label}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub struct Connection(pub Client);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.disconnect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.0.get_state() == ConnectionState::Connected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn connect(port: u16) -> (State, Connection) {
    connect_client(ClientBuilder::<State>::new(), port, |_| {})
}

/// Connects a client whose handles `prepare` may initialize before connecting.
fn connect_client(
    builder: ClientBuilder<State>,
    port: u16,
    prepare: impl FnOnce(&State),
) -> (State, Connection) {
    let hash = builder.get_version_hash();
    let (state, client) = builder.version(hash).build(port);
    prepare(&state);
    let connection = Connection(client);
    wait("client connection", || {
        connection.0.connect();
        (connection.0.get_state() == ConnectionState::Connected).then_some(())
    });
    wait("initial synchronized value", || {
        (state.integration.value.get() == 7).then_some(())
    });
    (state, connection)
}

fn once<T: PartialEq + std::fmt::Debug>(
    label: &str,
    expected: T,
    mut take: impl FnMut() -> Option<T>,
) {
    assert_eq!(wait(label, &mut take), expected, "{label}");
    assert_eq!(take(), None, "{label} consumed more than once");
}

fn phase(s: &IntegrationState, n: u32) {
    wait("server phase", || (s.phase.get() == n).then_some(()));
}

fn barrier(s: &IntegrationState) {
    s.command.set(99u32);
    phase(s, 99);
}

pub fn run(port: u16, scenario: &str) {
    match scenario {
        "images-wire" => return images_wire(port),
        "takeover" => return takeover(port),
        "rejected-client" => return rejected_client(port),
        _ => {}
    }
    let (state, connection) = connect(port);
    let s = &state.integration;
    match scenario {
        "sync" => {
            wait("initial collection", || {
                s.items.read(|v| (v == &[1, 2]).then_some(()))
            });
            s.command.set(1u32);
            wait("server update", || (s.value.get() == 21).then_some(()));
            s.value.set_signal(42);
            wait("value callback", || {
                (s.callback_value.get() == 42).then_some(())
            });
            s.command.set(2u32);
            wait("collection callback", || {
                s.items.read(|v| (v == &[1, 2, 42]).then_some(()))
            });
            wait("map callback", || {
                s.map.read(|v| (v.get(&9) == Some(&900)).then_some(()))
            });
        }
        "takes" => {
            s.command.set(3u32);
            once("float take", vec![-1.25, 0.0, 2.5], || {
                state.data_take.take_samples.take()
            });
            once("sparse float take", vec![9.5, -2.25], || {
                state.data_multi_take.samples.take(513)
            });
            once("nested take", vec![0, u16::MAX], || {
                state.data_multi_take.nested.buffer.take(24)
            });
            once("value payload", String::from("payload"), || s.take.take());
            once("unit payload", (), || s.empty.take());
            once("data payload", vec![0, 1, 255], || s.data.take());
            once("sparse multi payload", vec![100, 200], || s.multi.take(7));
            once("empty sparse payload", Vec::<u16>::new(), || {
                s.multi.take(42)
            });
            assert!(s.multi.take(0).is_none());
            s.command.set(4u32);
            once("empty string", String::new(), || s.take.take());
            once("empty data", Vec::<u8>::new(), || s.data.take());
            barrier(s);
            assert!(s.take.take().is_none());
            assert!(s.empty.take().is_none());
            assert!(s.data.take().is_none());
            assert!(s.multi.take(7).is_none());
            assert!(s.multi.take(42).is_none());
            drop(connection);
            let (next, _connection) = connect(port);
            barrier(&next.integration);
            assert!(next.integration.take.take().is_none());
            assert!(next.integration.empty.take().is_none());
            assert!(next.integration.data.take().is_none());
            assert!(next.integration.multi.take(7).is_none());
            assert!(next.data_take.take_samples.take().is_none());
            assert!(next.data_multi_take.nested.buffer.take(24).is_none());
        }
        "blocking-value" | "blocking-empty" | "blocking-data" | "blocking-multi" => {
            let command: u32 = match scenario {
                "blocking-value" => 10,
                "blocking-empty" => 11,
                "blocking-data" => 12,
                _ => 13,
            };
            s.command.set(command);
            phase(s, 1);
            // A negative assertion over a bounded interval proves the subsequent
            // send remains pending while the first payload is unconsumed.
            let until = Instant::now() + Duration::from_millis(150);
            while Instant::now() < until {
                assert_eq!(s.phase.get(), 1, "send completed before consumption");
                std::thread::sleep(Duration::from_millis(5));
            }
            match command {
                10 => assert_eq!(wait("first value", || s.take.take()), "first"),
                11 => {
                    wait("first unit", || s.empty.take());
                }
                12 => assert_eq!(wait("first data", || s.data.take()), vec![1, 2]),
                _ => assert_eq!(wait("first multi", || s.multi.take(73)), vec![100, 200]),
            }
            phase(s, 2);
            match command {
                10 => once("second value", String::new(), || s.take.take()),
                11 => once("second unit", (), || s.empty.take()),
                12 => once("second data", Vec::<u8>::new(), || s.data.take()),
                _ => once("second multi", Vec::<u16>::new(), || s.multi.take(73)),
            }
            barrier(s);
        }
        "cache" => {
            once("cached data", vec![8, 9], || s.cached.take());
            once("cached sparse data", vec![500, 600], || {
                s.cached_multi.take(91)
            });
            drop(connection);
            let (next, _connection) = connect(port);
            once("cached data for new client", vec![8, 9], || {
                next.integration.cached.take()
            });
            once("cached sparse data for new client", vec![500, 600], || {
                next.integration.cached_multi.take(91)
            });
        }
        "wire" => {
            use egui_states_test_schema::sections::{TestEnum2, TestStruct2, WirePacket};
            let expected = WirePacket {
                signed: i64::MIN,
                unsigned: u64::MAX,
                text: "žluťoučký 🦀".into(),
                optional: Some(i32::MIN),
                fixed: [0, 32768, u16::MAX],
                nested: TestStruct2 {
                    enabled: true,
                    level: u16::MAX,
                    name: "中".into(),
                },
                choice: TestEnum2::Z,
            };
            wait("non-default packet synchronization", || {
                (s.wire.get() == expected).then_some(())
            });
            for (text, optional, choice) in [
                ("", None, TestEnum2::X),
                ("λ🦀", Some(i32::MAX), TestEnum2::Y),
            ] {
                let packet = WirePacket {
                    signed: i64::MAX,
                    unsigned: 0,
                    text: text.into(),
                    optional,
                    fixed: [u16::MAX, 0, 1],
                    nested: TestStruct2 {
                        enabled: false,
                        level: 0,
                        name: "".into(),
                    },
                    choice,
                };
                s.wire.set_signal(packet.clone());
                wait("server decoded and echoed complete packet", || {
                    (s.wire_echo.get() == packet).then_some(())
                });
            }
        }
        "take-controls" => {
            s.command.set(22u32);
            phase(s, 22);
            assert!(
                s.multi.take(7).is_none(),
                "removed payload remains consumable"
            );
            once("unrelated sparse key survives removal", vec![9], || {
                s.multi.take(900)
            });
            s.command.set(22u32);
            barrier(s);
            s.command.set(23u32);
            phase(s, 23);
            assert!(
                s.multi.take(900).is_none(),
                "reset payload remains consumable"
            );
            s.command.set(22u32);
            phase(s, 22);
            once("progress after reset", vec![9], || s.multi.take(900));
        }
        "cache-replacement" => {
            s.command.set(20u32);
            phase(s, 20);
            drop(connection);
            let (next, next_connection) = connect(port);
            let s = &next.integration;
            barrier(s);
            once("replacement cache", vec![31, 32], || s.cached.take());
            once("replacement sparse cache", vec![17], || {
                s.cached_multi.take(513)
            });
            assert!(s.cached_multi.take(91).is_none());
            s.command.set(21u32);
            phase(s, 21);
            once("uncached payload", vec![41], || s.cached.take());
            drop(next_connection);
            let (last, _last_connection) = connect(port);
            barrier(&last.integration);
            assert!(
                last.integration.cached.take().is_none(),
                "uncached send must clear old cache"
            );
            assert!(last.integration.cached_multi.take(513).is_none());
        }
        "blocking-remove" | "blocking-reset" => {
            s.command.set(if scenario == "blocking-remove" {
                24u32
            } else {
                25u32
            });
            phase(s, 24);
            assert!(s.multi.take(7).is_none());
            wait(
                "next key progresses after discarding pending blocking payload",
                || (s.phase.get() == 25).then_some(()),
            );
            once("next key", vec![9], || s.multi.take(513));
        }
        "disconnect-pending" => {
            s.command.set(12u32);
            phase(s, 1);
            drop(connection);
            // The second blocking send must release on disconnect without replay.
            let (next, _connection) = connect(port);
            barrier(&next.integration);
            assert!(next.integration.data.take().is_none());
        }
        "values-wire" => values_wire(&state),
        "collections-wire" => collections_wire(&state),
        "numeric-wire" => numeric_wire(&state),
        _ => panic!("unknown scenario: {scenario}"),
    }
}

/// Every value shape crosses the wire in both directions. The servers set the
/// values on command 30 and compare the client's values on command 31.
fn values_wire(state: &State) {
    use egui_states_test_schema::sections::{TestEnum, TestEnum2, TestStruct, TestStruct2};
    use std::collections::HashMap;

    let s = &state.integration;
    let (v, c, st, w) = (
        &state.values,
        &state.custom_values,
        &state.statics,
        &state.wire_types,
    );
    s.command.set(30u32);
    phase(s, 30);
    assert!(v.bool_value.get());
    assert_eq!(v.count.get(), -123_456);
    assert_eq!(v.ratio.get(), 0.375);
    assert_eq!(v.queued_progress.get(), 0.625);
    assert_eq!(v.title.get(), "ünïcødé 🦀");
    assert_eq!(v.optional_value.get(), Some(-5));
    assert_eq!(v.fixed_numbers.get(), [1, 65535, 0]);
    assert_eq!(v.test_enum.get(), TestEnum::C);
    assert_eq!(v.nested.secondary_choice.get(), TestEnum2::X);
    assert_eq!(v.nested.selected_enum.get(), Some(TestEnum::B));
    assert_eq!(
        c.point.get(),
        TestStruct {
            x: -1.5,
            y: 2.25,
            label: "point".into()
        }
    );
    assert_eq!(
        c.optional_struct.get(),
        Some(TestStruct2 {
            enabled: true,
            level: 7,
            name: "seven".into()
        })
    );
    assert_eq!(st.status_text.get(), "ready");
    assert_eq!(
        st.summary.get(),
        TestStruct2 {
            enabled: false,
            level: 65535,
            name: "max".into()
        }
    );
    assert_eq!(st.pair.get(), [0.5, -0.25]);
    assert_eq!(st.nested.label.get(), "label");
    assert_eq!(st.nested.enum_hint.get(), TestEnum::C);
    assert_eq!(w.small_unsigned.get(), 255);
    assert_eq!(w.small_signed.get(), -128);
    assert_eq!(w.short.get(), -32768);
    assert_eq!(w.wide.get(), u64::MAX);
    assert_eq!(w.single.get(), -0.125);
    assert_eq!(w.tuple.get(), (-7, "tuple".to_string(), true));
    assert_eq!(
        w.list.get(),
        [
            TestStruct2 {
                enabled: true,
                level: 1,
                name: "a".into()
            },
            TestStruct2 {
                enabled: false,
                level: 2,
                name: "b".into()
            }
        ]
    );
    assert_eq!(
        w.table.get(),
        HashMap::from([("pi".to_string(), 3.5), ("neg".to_string(), -0.5)])
    );
    assert_eq!(
        w.keyed_enum.get(),
        HashMap::from([(TestEnum::A, 1), (TestEnum::C, 3)])
    );
    assert_eq!(w.optional_enum.get(), Some(TestEnum2::Z));
    assert_eq!(w.bytes.get(), [0, 127, 255]);

    v.bool_value.set(false);
    v.count.set(654_321);
    v.ratio.set(-0.75);
    v.queued_progress.set(0.125);
    v.title.set("client ✓".into());
    v.optional_value.set(None);
    v.fixed_numbers.set([9, 8, 7]);
    v.test_enum.set(TestEnum::A);
    v.nested.secondary_choice.set(TestEnum2::Y);
    v.nested.selected_enum.set(None);
    c.point.set(TestStruct {
        x: 0.5,
        y: -0.5,
        label: "client".into(),
    });
    c.optional_struct.set(None);
    w.small_unsigned.set(1);
    w.small_signed.set(127);
    w.short.set(32767);
    w.wide.set(0);
    w.single.set(0.5);
    w.tuple.set((0, String::new(), false));
    w.list.set(Vec::new());
    w.table.set(HashMap::new());
    w.keyed_enum.set(HashMap::from([(TestEnum::B, 2)]));
    w.optional_enum.set(None);
    state.signals.empty_signal.set(());
    state.signals.number_signal.set(-2.5);
    state.signals.enum_signal.set(TestEnum::C);
    w.pair_signal.set((1.5, TestEnum::B));
    // The servers record a mismatch as an error, which fails the test harness.
    s.command.set(31u32);
    phase(s, 31);
}

fn collections_wire(state: &State) {
    let s = &state.integration;
    s.command.set(32u32);
    phase(s, 32);
    assert_eq!(state.value_vec.items.get(), [60, 7, 8]);
    assert_eq!(
        state.value_map.items.get(),
        std::collections::HashMap::from([(1, 11), (3, 30)])
    );
}

fn numeric_wire(state: &State) {
    let s = &state.integration;
    s.command.set(33u32);
    phase(s, 33);
    let (d, n, m) = (&state.data, &state.numeric, &state.multi_data);
    assert_eq!(d.bytes.get(), [20, 30, 4, 5, 6]);
    assert_eq!(d.samples.get(), [-1.5, 0.25]);
    assert!(d.nested.buffer.get().is_empty());
    assert_eq!(n.u8_data.get(), [u8::MIN, 0, u8::MAX]);
    assert_eq!(n.i8_data.get(), [i8::MIN, 0, i8::MAX]);
    assert_eq!(n.u16_data.get(), [u16::MIN, 0, u16::MAX]);
    assert_eq!(n.i16_data.get(), [i16::MIN, 0, i16::MAX]);
    assert_eq!(n.u32_data.get(), [u32::MIN, 0, u32::MAX]);
    assert_eq!(n.i32_data.get(), [i32::MIN, 0, i32::MAX]);
    assert_eq!(n.u64_data.get(), [u64::MIN, 0, u64::MAX]);
    assert_eq!(n.i64_data.get(), [i64::MIN, 0, i64::MAX]);
    assert_eq!(n.f32_data.get(), [f32::MIN, 0.0, f32::MAX]);
    assert_eq!(n.f64_data.get(), [f64::MIN, 0.0, f64::MAX]);
    assert_eq!(m.bytes.get(1), Some(vec![1, 2, 3]));
    assert_eq!(m.bytes.get(2), None);
    assert_eq!(m.samples.get(7), Some(vec![0.5]));
    assert_eq!(m.nested.buffer.get(3), Some(vec![4, 6]));
}

/// Applies every queued texture change to `textures`, like a painter would.
fn apply_texture_deltas(
    context: &egui::Context,
    textures: &mut std::collections::HashMap<egui::TextureId, egui::ColorImage>,
) {
    let mut delta = context.tex_manager().write().take_delta();
    for (id, changes) in delta.set.drain() {
        for change in changes {
            let egui::ImageData::Color(image) = &change.image;
            match change.pos {
                None => {
                    textures.insert(id, (**image).clone());
                }
                Some([x, y]) => {
                    let texture = textures.get_mut(&id).expect("patch for a missing texture");
                    for row in 0..image.size[1] {
                        for column in 0..image.size[0] {
                            texture.pixels[(y + row) * texture.size[0] + x + column] =
                                image.pixels[row * image.size[0] + column];
                        }
                    }
                }
            }
        }
    }
    // The deltas were applied above instead of by a painter.
    delta.clear();
}

fn rgba(texture: &egui::ColorImage) -> Vec<[u8; 4]> {
    texture
        .pixels
        .iter()
        .map(|pixel| pixel.to_array())
        .collect()
}

/// Server images reach the client textures, including partial updates and
/// sparse key removal. The servers use opaque pixels only.
fn images_wire(port: u16) {
    let context = egui::Context::default();
    let (state, _connection) = connect_client(
        ClientBuilder::<State>::new().context(context.clone()),
        port,
        |state| {
            state.image.image.initialize(
                &context,
                egui::ColorImage::filled([1, 1], egui::Color32::BLACK),
            );
            state.image.images.initialize(&context);
        },
    );
    let s = &state.integration;
    s.command.set(34u32);
    phase(s, 34);

    assert_eq!(state.image.image.get_size(), Some([3, 2]));
    assert_eq!(state.image.images.indices(), [7]);
    assert_eq!(state.image.images.get_size(7), Some([2, 2]));

    let mut textures = std::collections::HashMap::new();
    apply_texture_deltas(&context, &mut textures);
    let pixel = |r: u8| [r, r + 1, r + 2, 255];
    let image = &textures[&state.image.image.get_id().unwrap()];
    assert_eq!(image.size, [3, 2]);
    assert_eq!(
        rgba(image),
        [
            pixel(0),
            pixel(3),
            pixel(6),
            pixel(9),
            pixel(200),
            pixel(210)
        ]
    );
    let sparse = &textures[&state.image.images.get_id(7).unwrap()];
    assert_eq!(rgba(sparse), [[1, 2, 3, 255]; 4]);
}

/// A second client replaces the first one, which observes the disconnect.
fn takeover(port: u16) {
    let (_first_state, first) = connect(port);
    let (second_state, _second) = connect(port);
    wait("replaced client disconnect", || {
        (first.0.get_state() == ConnectionState::Disconnected).then_some(())
    });
    barrier(&second_state.integration);
}

/// A client with a mismatched layout version is refused without affecting the
/// connected client. The servers check that the refusal was logged.
fn rejected_client(port: u16) {
    let (state, connection) = connect(port);
    let builder = ClientBuilder::<State>::new();
    let wrong_version = builder.get_version_hash() ^ 1;
    let (rejected, client) = builder.version(wrong_version).build(port);
    let rejected_connection = Connection(client);
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        rejected_connection.0.connect();
        assert_eq!(
            rejected.integration.value.get(),
            0,
            "a rejected client received synchronized state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    barrier(&state.integration);
    assert!(connection.0.get_state() == ConnectionState::Connected);
}
