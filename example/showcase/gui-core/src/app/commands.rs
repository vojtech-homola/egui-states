use crate::state::commands::{Command, Status};
use crate::state::sections::{PrimaryChoice, Summary};

use super::{ViewState, helpers::primary_choice_label};

/// The parameters the GUI edits before it sends a command.
pub(super) struct CommandInputs {
    x: f32,
    y: f32,
    gain: f32,
    mark: u32,
    text: String,
    choice: PrimaryChoice,
    path: String,
    with_summary: bool,
    level: u16,
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
            path: "sample.tif".to_string(),
            with_summary: true,
            level: 3,
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
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut inputs.path).desired_width(120.0));
            ui.checkbox(&mut inputs.with_summary, "summary");
            ui.add_enabled(
                inputs.with_summary,
                egui::DragValue::new(&mut inputs.level).prefix("level "),
            );
            if ui.button("Load { path, summary }").clicked() {
                send.push(Command::Load {
                    path: inputs.path.clone(),
                    summary: inputs.with_summary.then(|| Summary {
                        enabled: true,
                        level: inputs.level,
                        name: inputs.text.clone(),
                    }),
                });
            }
        });
        ui.label("The server fails a load without a summary.");
        ui.horizontal(|ui| {
            ui.label("Report(status):");
            if ui.button("Idle").clicked() {
                send.push(Command::Report(Status::Idle));
            }
            if ui.button("At { x, y }").clicked() {
                send.push(Command::Report(Status::At {
                    x: inputs.x,
                    y: inputs.y,
                }));
            }
            if ui.button("Failed(text)").clicked() {
                send.push(Command::Report(Status::Failed(inputs.text.clone())));
            }
        });
        ui.label("The server answers a report with the status it carries.");
        if ui.button("send all seven at once").clicked() {
            inputs.mark += 1;
            send.extend([
                Command::MoveTo { x: 5.0, y: 5.0 },
                Command::SetGain(0.5),
                Command::Mark(inputs.mark, "burst".to_string()),
                Command::Choose(PrimaryChoice::C),
                Command::Load {
                    path: "burst.tif".to_string(),
                    summary: None,
                },
                Command::Report(Status::Failed("burst".to_string())),
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
        ui.label("VecState<HandledCommand>: root.commands.history, newest last");
        c.history.read(|history| {
            if history.is_empty() {
                ui.label("no command yet");
            }
            for handled in history {
                ui.label(format!(
                    "{} -> {}",
                    describe_command(&handled.command),
                    describe_status(&handled.status)
                ));
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
        Command::Load { path, summary } => match summary {
            Some(summary) => format!(
                "Load {{ path: {path:?}, summary: Some({}, level {}) }}",
                summary.name, summary.level
            ),
            None => format!("Load {{ path: {path:?}, summary: None }}"),
        },
        Command::Report(status) => format!("Report({})", describe_status(status)),
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
