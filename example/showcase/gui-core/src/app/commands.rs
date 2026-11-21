use crate::state::commands::{Command, Status};
use crate::state::sections::PrimaryChoice;

use super::{ViewState, helpers::primary_choice_label};

/// The parameters the GUI edits before it sends a command.
pub(super) struct CommandInputs {
    x: f32,
    y: f32,
    gain: f32,
    mark: u32,
    text: String,
    choice: PrimaryChoice,
}

impl Default for CommandInputs {
    fn default() -> Self {
        Self {
            x: 1.0,
            y: 2.0,
            gain: 1.0,
            mark: 0,
            text: "note".to_string(),
            choice: PrimaryChoice::B,
        }
    }
}

pub(super) fn show_commands(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("enums with data", |ui| {
        let c = &state.schema.commands;
        let inputs = &mut state.command_inputs;

        ui.label("Signal<Command>, #[state(queue)]: root.commands.command");
        ui.label("Every command reaches the server in the order it was sent.");
        let mut send = Vec::new();
        ui.horizontal(|ui| {
            if ui.button("Stop").clicked() {
                send.push(Command::Stop);
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut inputs.x).speed(0.5).prefix("x "));
            ui.add(egui::DragValue::new(&mut inputs.y).speed(0.5).prefix("y "));
            let move_to = Command::MoveTo {
                x: inputs.x,
                y: inputs.y,
            };
            if ui.button("MoveTo { x, y }").clicked() {
                send.push(move_to.clone());
            }
            if ui.button("save as preset").clicked() {
                c.preset.set(move_to);
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut inputs.gain, 0.0..=4.0).text("gain"));
            if ui.button("SetGain(gain)").clicked() {
                send.push(Command::SetGain(inputs.gain));
            }
        });
        ui.label("The server fails a gain above 2.");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut inputs.text).desired_width(120.0));
            if ui.button("Mark(number, text)").clicked() {
                inputs.mark += 1;
                send.push(Command::Mark(inputs.mark, inputs.text.clone()));
            }
        });
        ui.horizontal(|ui| {
            for choice in [PrimaryChoice::A, PrimaryChoice::B, PrimaryChoice::C] {
                ui.selectable_value(&mut inputs.choice, choice, primary_choice_label(choice));
            }
            if ui.button("Choose(choice)").clicked() {
                send.push(Command::Choose(inputs.choice));
            }
        });
        if ui.button("send all five at once").clicked() {
            inputs.mark += 1;
            send.extend([
                Command::MoveTo { x: 5.0, y: 5.0 },
                Command::SetGain(0.5),
                Command::Mark(inputs.mark, "burst".to_string()),
                Command::Choose(PrimaryChoice::C),
                Command::Stop,
            ]);
        }
        for command in send {
            c.command.set(command);
        }

        ui.separator();
        ui.label(format!(
            "Static<Status>: root.commands.status = {}",
            describe_status(&c.status.get())
        ));
        ui.label("VecState<Command>: root.commands.history, newest last");
        c.history.read(|history| {
            if history.is_empty() {
                ui.label("no command yet");
            }
            for command in history {
                ui.label(describe_command(command));
            }
        });

        ui.separator();
        ui.label("Value<Command>, initially a variant with data: root.commands.preset");
        let preset = c.preset.get();
        ui.horizontal(|ui| {
            ui.label(describe_command(&preset));
            if ui.button("send preset").clicked() {
                c.command.set(preset.clone());
            }
        });
    });
}

fn describe_command(command: &Command) -> String {
    match command {
        Command::Stop => "Stop".to_string(),
        Command::MoveTo { x, y } => format!("MoveTo {{ x: {x}, y: {y} }}"),
        Command::SetGain(gain) => format!("SetGain({gain:.2})"),
        Command::Mark(number, text) => format!("Mark({number}, {text:?})"),
        Command::Choose(choice) => format!("Choose({})", primary_choice_label(*choice)),
    }
}

fn describe_status(status: &Status) -> String {
    match status {
        Status::Idle => "Idle".to_string(),
        Status::At { x, y } => format!("At {{ x: {x}, y: {y} }}"),
        Status::Handled(count) => format!("Handled({count})"),
        Status::Failed(reason) => format!("Failed({reason:?})"),
    }
}
