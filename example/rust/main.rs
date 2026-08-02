use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use egui_states::server as s;
use rust_server_example::states_server::StatesServer;
use rust_server_example::states_server::enums::{TestEnum, TestEnum2};
use rust_server_example::states_server::structs::{TestStruct, TestStruct2};

const PORT: u16 = 8091;
const DEFAULT_VEC: [i32; 3] = [10, -3, 27];

fn default_map() -> HashMap<u16, u32> {
    HashMap::from([(1, 100), (2, 200), (5, 500)])
}

fn main() -> s::Result<()> {
    let server = StatesServer::new(PORT)?;
    let states = &server.states;

    let mut callbacks = Vec::new();
    callbacks.push(server.logging.add_logger(s::LogLevel::Debug, |message| {
        println!("Debug: {message}");
    }));
    callbacks.push(server.logging.add_logger(s::LogLevel::Info, |message| {
        println!("Info: {message}");
    }));
    callbacks.push(server.logging.add_logger(s::LogLevel::Warning, |message| {
        println!("Warning: {message}");
    }));
    callbacks.push(server.logging.add_logger(s::LogLevel::Error, |message| {
        println!("Error: {message}");
    }));

    callbacks.push(states.values.ratio.connect(|value| {
        println!("ratio changed: {value:.3}");
    }));
    callbacks.push(states.values.title.connect(|value| {
        println!("title changed: {value}");
    }));
    callbacks.push(states.values.count.connect_previous(|value, previous| {
        println!("count changed: {previous} -> {value}");
    }));
    callbacks.push(states.values.test_enum.connect(|value| {
        println!("enum changed: {value:?}");
    }));
    callbacks.push(states.signals.empty_signal.connect_empty(|| {
        println!("empty signal emitted");
    }));
    callbacks.push(states.signals.number_signal.connect(|value| {
        println!("number signal emitted: {value:.3}");
    }));
    callbacks.push(states.signals.enum_signal.connect(|value| {
        println!("enum signal emitted: {value:?}");
    }));

    let value_vec = states.value_vec.items.clone();
    callbacks.push(states.value_vec.actions.reset_demo.connect_empty(move || {
        if let Err(error) = value_vec.set(DEFAULT_VEC.to_vec(), true) {
            eprintln!("failed to reset value_vec: {error}");
        }
    }));

    let value_vec = states.value_vec.items.clone();
    callbacks.push(states.value_vec.actions.append_item.connect_empty(
        move || match value_vec.get() {
            Ok(current) => {
                let next_value = current.last().copied().unwrap_or(DEFAULT_VEC[0]) + 5;
                if let Err(error) = value_vec.add_item(next_value, true) {
                    eprintln!("failed to append value_vec item: {error}");
                } else {
                    println!("value_vec appended: {next_value}");
                }
            }
            Err(error) => eprintln!("failed to read value_vec: {error}"),
        },
    ));

    let value_vec = states.value_vec.items.clone();
    callbacks.push(states.value_vec.actions.remove_last.connect_empty(move || {
        match value_vec.len().checked_sub(1) {
            Some(index) => {
                if let Err(error) = value_vec.remove_item(index, true) {
                    eprintln!("failed to remove value_vec item: {error}");
                } else {
                    println!("value_vec removed last item");
                }
            }
            None => {}
        }
    }));

    let value_map = states.value_map.items.clone();
    callbacks.push(states.value_map.actions.reset_demo.connect_empty(move || {
        if let Err(error) = value_map.set(default_map(), true) {
            eprintln!("failed to reset value_map: {error}");
        }
    }));

    let value_map = states.value_map.items.clone();
    callbacks.push(states.value_map.actions.insert_next.connect_empty(
        move || match value_map.get() {
            Ok(current) => {
                let next_key = current.keys().copied().max().unwrap_or(0) + 1;
                let next_value = u32::from(next_key) * 100;
                if let Err(error) = value_map.set_item(next_key, next_value, true) {
                    eprintln!("failed to insert value_map item: {error}");
                } else {
                    println!("value_map inserted: {next_key} -> {next_value}");
                }
            }
            Err(error) => eprintln!("failed to read value_map: {error}"),
        },
    ));

    let value_map = states.value_map.items.clone();
    callbacks.push(
        states
            .value_map
            .actions
            .remove_lowest
            .connect_empty(move || match value_map.get() {
                Ok(current) => {
                    if let Some(lowest_key) = current.keys().copied().min() {
                        if let Err(error) = value_map.remove_item(&lowest_key, true) {
                            eprintln!("failed to remove value_map item: {error}");
                        } else {
                            println!("value_map removed key: {lowest_key}");
                        }
                    }
                }
                Err(error) => eprintln!("failed to read value_map: {error}"),
            }),
    );

    states.values.bool_value.set(true, false)?;
    states.values.count.set(7, false)?;
    states.values.ratio.set_signal(0.42, false)?;
    states.values.queued_progress.set(0.25, false)?;
    states
        .values
        .title
        .set_signal(String::from("Interactive egui-states Rust example"), false)?;
    states.values.optional_value.set(Some(12), false)?;
    states.values.fixed_numbers.set([2, 4, 8], false)?;
    states.values.test_enum.set_signal(TestEnum::C, false)?;
    states
        .values
        .nested
        .secondary_choice
        .set(TestEnum2::Z, false)?;
    states
        .values
        .nested
        .selected_enum
        .set(Some(TestEnum::B), false)?;

    states
        .statics
        .status_text
        .set(String::from("Static values are shown as labels."), false)?;
    states.statics.summary.set(
        TestStruct2 {
            enabled: true,
            level: 3,
            name: String::from("static summary"),
        },
        false,
    )?;
    states.statics.pair.set([0.5, 1.5], false)?;
    states
        .statics
        .nested
        .label
        .set(String::from("Nested static label"), false)?;
    states.statics.nested.enum_hint.set(TestEnum::A, false)?;

    states.custom_values.point.set(
        TestStruct {
            x: 1.5,
            y: -0.75,
            label: String::from("editable point"),
        },
        false,
    )?;
    states.custom_values.optional_struct.set(
        Some(TestStruct2 {
            enabled: true,
            level: 9,
            name: String::from("optional payload"),
        }),
        false,
    )?;

    states.value_vec.items.set(DEFAULT_VEC.to_vec(), true)?;
    states.value_map.items.set(default_map(), true)?;

    let bytes = (0u8..32).collect::<Vec<_>>();
    states.data.bytes.set(&bytes, true)?;
    let samples = (0..(1024 * 20))
        .map(|index| index as f32 / ((1024 * 20 - 1) as f32))
        .collect::<Vec<_>>();
    states.data.samples.set(&samples, true)?;
    let nested_buffer = (0u16..8).collect::<Vec<_>>();
    states.data.nested.buffer.set(&nested_buffer, true)?;

    states
        .multi_data
        .bytes
        .set(0, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], true)?;
    states.multi_data.bytes.replace(0, 4, &[200, 201], true)?;
    states.multi_data.bytes.set(1, &[10, 20, 30], true)?;
    states.multi_data.bytes.add(1, &[40, 50], true)?;
    states
        .multi_data
        .samples
        .set(0, &[0.0, 0.25, 0.5, 0.75, 1.0], true)?;
    states
        .multi_data
        .samples
        .set(2, &[-1.0, -0.5, 0.0, 0.5, 1.0], true)?;
    states.multi_data.samples.add(2, &[1.5, 2.0], true)?;
    states
        .multi_data
        .nested
        .buffer
        .set(0, &[0, 1, 2, 3], true)?;
    states.multi_data.nested.buffer.add(0, &[10, 11], true)?;
    states
        .multi_data
        .nested
        .buffer
        .set(3, &[100, 110, 120], true)?;

    states
        .value_take
        .take_text
        .set(String::from("ValueTake payload from Rust"), false, true)?;
    states.value_take.take_empty.set((), false, true)?;

    let take_buffer = (0u8..16).collect::<Vec<_>>();
    states
        .data_take
        .take_buffer
        .set(&take_buffer, false, true, true)?;
    let take_samples = (0..8).map(|index| index as f32 / 7.0).collect::<Vec<_>>();
    states
        .data_take
        .take_samples
        .set(&take_samples, false, true, true)?;

    states
        .data_multi_take
        .bytes
        .set(0, &[0, 1, 2, 3, 4, 5], false, true, true)?;
    states
        .data_multi_take
        .bytes
        .set(1, &[10, 20, 30], false, true, true)?;
    states
        .data_multi_take
        .samples
        .set(0, &[0.0, 0.33, 0.66, 1.0], false, true, true)?;
    states
        .data_multi_take
        .samples
        .set(2, &[-1.0, 0.0, 1.0], false, true, true)?;
    states
        .data_multi_take
        .nested
        .buffer
        .set(0, &[0, 1, 2, 3], false, true, true)?;
    states
        .data_multi_take
        .nested
        .buffer
        .set(3, &[100, 110], false, true, true)?;

    let mut image = vec![0u8; 256 * 256 * 3];
    for y in 0..256usize {
        for x in 0..256usize {
            let offset = (y * 256 + x) * 3;
            image[offset] = x as u8;
            image[offset + 1] = y as u8;
            image[offset + 2] = ((x + y) / 2) as u8;
        }
    }
    states
        .image
        .image
        .set(&image, [256, 256], s::ImageFormat::Color, true)?;

    let mut image_patch = vec![0u8; 64 * 64 * 4];
    for pixel in image_patch.chunks_exact_mut(4) {
        pixel[1] = 220;
        pixel[3] = 255;
    }
    states.image.image.update(
        &image_patch,
        [96, 96],
        [64, 64],
        s::ImageFormat::ColorAlpha,
        true,
        false,
    )?;

    server.start()?;
    println!("Rust egui-states server listening on port {PORT}");

    loop {
        thread::sleep(Duration::from_secs(1));
        let _ = &callbacks;
    }
}
