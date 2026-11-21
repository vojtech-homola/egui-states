//! Commands with parameters, sent in order through one queued signal, and the
//! status the server answers with: enums with data.
use egui_states::{Signal, Static, Value, VecState};

use super::sections::PrimaryChoice;

/// A command for a simulated stage. Its variants show every shape an enum
/// with data can have.
#[egui_states::typed(rust_derive(Debug))]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
pub enum Command {
    /// A unit variant.
    #[default]
    Stop,
    /// A variant with named fields.
    MoveTo { x: f32, y: f32 },
    /// A tuple variant with one field.
    SetGain(f32),
    /// A tuple variant with two fields: a mark number and its text.
    Mark(u32, String),
    /// A field of a fieldless enum.
    Choose(PrimaryChoice),
}

/// The stage's status after the last command, set by the server.
#[egui_states::typed(rust_derive(Debug))]
#[derive(Clone, Default, PartialEq, egui_states::InitialValue)]
pub enum Status {
    #[default]
    Idle,
    At {
        x: f32,
        y: f32,
    },
    /// How many commands the server has handled.
    Handled(u32),
    Failed(String),
}

#[derive(egui_states::State)]
pub struct CommandStates {
    /// Every command, in the order the GUI sent it.
    #[state(queue)]
    pub command: Signal<Command>,
    /// Starts from a variant with data, as does `preset`.
    #[state(init = Status::At { x: 0.0, y: 0.0 })]
    pub status: Static<Status>,
    /// The latest commands the server handled, newest last.
    pub history: VecState<Command>,
    /// A command the GUI edits and sends.
    #[state(init = Command::MoveTo { x: 10.0, y: 20.0 })]
    pub preset: Value<Command>,
}
