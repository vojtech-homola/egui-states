use egui_states::{State, StatesCreator, Value};
use std::collections::HashMap;

#[egui_states::typed(rust_derive(
    Clone,
    Debug,
    PartialEq,
    egui_states::Typed,
    egui_states::serde::Serialize,
    custom_macros::Proof
))]
#[derive(Clone, Default, egui_states::InitialValue)]
pub struct Payload {
    pub title: String,
    pub nested: Option<Inner>,
    pub fixed: [u16; 3],
    pub choice: Choice,
}
#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Default, egui_states::InitialValue)]
pub struct Inner {
    pub enabled: bool,
}
#[egui_states::typed(rust_derive(Debug))]
#[derive(Clone, Copy, Default, egui_states::InitialValue)]
pub enum Choice {
    #[default]
    Low = 3,
    High = 71,
}

pub struct Root<const REVERSE: bool>;
impl<const REVERSE: bool> State for Root<REVERSE> {
    const NAME: &'static str = "Root";
    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Leaf = c.substate("first");
        let _: Leaf = c.substate("second");
        let _: Value<Payload> = c.value(
            "payload",
            Payload {
                title: "hello 🦀".into(),
                nested: Some(Inner { enabled: true }),
                fixed: [0, 17, u16::MAX],
                choice: Choice::High,
            },
        );
        let mut entries = vec![(1u16, 10u32), (20, 200), (3, 30)];
        if REVERSE {
            entries.reverse();
        }
        let _: Value<HashMap<u16, u32>> = c.value("mapping", entries.into_iter().collect());
        Self
    }
}

#[cfg(feature = "conflicting")]
pub mod conflicting {
    use super::*;
    pub struct Leaf<const INITIAL: bool>;
    impl<const INITIAL: bool> State for Leaf<INITIAL> {
        const NAME: &'static str = "Leaf";
        fn new(c: &mut impl StatesCreator) -> Self {
            let _: Value<bool> = c.value("value", INITIAL);
            Self
        }
    }
    pub struct Conflict;
    impl State for Conflict {
        const NAME: &'static str = "Conflict";
        fn new(c: &mut impl StatesCreator) -> Self {
            let _: Leaf<false> = c.substate("first");
            let _: Leaf<true> = c.substate("second");
            Self
        }
    }
}

// Reusing a state containing a HashMap used to fail nondeterministically.
pub struct Leaf;
impl State for Leaf {
    const NAME: &'static str = "Leaf";
    fn new(c: &mut impl StatesCreator) -> Self {
        let _: Value<HashMap<u16, u32>> = c.value(
            "numbers",
            HashMap::from([(10, 10), (1, 1), (20, 20), (2, 2), (3, 3)]),
        );
        Self
    }
}
