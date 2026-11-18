#![cfg(feature = "client")]

use egui_states::{ClientBuilder, Signal, State, Static, StaticAtomic, Value, ValueAtomic};

#[egui_states::typed]
#[derive(Clone, Copy, Debug, Default, PartialEq, egui_states::InitialValue)]
enum Mode {
    #[default]
    Manual,
    Auto,
}

/// Shares its name with the creator parameter of the generated `State::new`.
fn c() -> f32 {
    0.25
}

/// Forwards `$init` the way a `macro_rules!` generated state struct does.
macro_rules! forwarded {
    ($name:ident, $init:expr) => {
        #[derive(State)]
        struct $name {
            #[state(init = $init)]
            text: Value<String>,
        }
    };
}

forwarded!(Forwarded, "forwarded");

#[derive(State)]
#[allow(dead_code)]
struct Initials {
    #[state(init = 0.5)]
    gain: Value<f32>,
    #[state(init = c())]
    from_item: Value<f32>,
    #[state(init = "ready")]
    status: Static<String>,
    #[state(init = Mode::Auto)]
    mode: Value<Mode>,
    #[state(init = (1, "pair".to_owned()))]
    pair: Value<(u8, String)>,
    #[state(init = 7)]
    counter: ValueAtomic<u16>,
    #[state(init = true)]
    flag: StaticAtomic<bool>,
    #[state(queue, init = 3)]
    queued: Value<u8>,
    plain: Value<i32>,
    reset: Signal<()>,
    forwarded: Forwarded,
}

#[test]
fn init_options_set_the_client_values() {
    let (state, _client) = ClientBuilder::<Initials>::new(egui::Context::default(), 0).build();

    assert_eq!(state.gain.get(), 0.5);
    assert_eq!(state.from_item.get(), 0.25);
    assert_eq!(state.status.get(), "ready");
    assert_eq!(state.mode.get(), Mode::Auto);
    assert_eq!(state.pair.get(), (1, "pair".to_owned()));
    assert_eq!(state.counter.get(), 7);
    assert!(state.flag.get());
    assert_eq!(state.queued.get(), 3);
    assert_eq!(state.plain.get(), 0);
    assert_eq!(state.forwarded.text.get(), "forwarded");
}
