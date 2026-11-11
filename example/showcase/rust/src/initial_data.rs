use crate::{
    DEFAULT_VEC,
    bindings::{
        ShowcaseState,
        enums::{PrimaryChoice, SecondaryChoice},
        structs::{Measurement, Point, Summary},
    },
    default_map,
};
use egui_states::server as s;

pub fn populate_initial_data(states: &ShowcaseState) -> s::Result<()> {
    states.values.bool_value.set(true, false)?;
    states.values.count.set(7, false)?;
    states.values.ratio.set_signal(0.42, false)?;
    states.values.queued_progress.set(0.25, false)?;
    states
        .values
        .title
        .set_signal(String::from("Interactive egui-states showcase"), false)?;
    states.values.optional_value.set(Some(12), false)?;
    states.values.fixed_numbers.set([2, 4, 8], false)?;
    states
        .values
        .primary_choice
        .set_signal(PrimaryChoice::C, false)?;
    states
        .values
        .nested
        .secondary_choice
        .set(SecondaryChoice::Z, false)?;
    states
        .values
        .nested
        .selected_enum
        .set(Some(PrimaryChoice::B), false)?;

    states
        .statics
        .status_text
        .set(String::from("Static values are shown as labels."), false)?;
    states.statics.summary.set(
        Summary {
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
    states
        .statics
        .nested
        .enum_hint
        .set(PrimaryChoice::A, false)?;

    states.custom_values.point.set(
        Point {
            x: 1.5,
            y: -0.75,
            label: String::from("editable point"),
        },
        false,
    )?;
    states.custom_values.optional_struct.set(
        Some(Summary {
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
    let temperatures = (0..50)
        .map(|index| 20.0 + (index as f32 / 8.0).sin())
        .collect::<Vec<_>>();
    let measurement = Measurement {
        label: "temperature".into(),
        unit: "°C".into(),
        sample_rate: 10.0,
    };
    states
        .data
        .measurement
        .set_with_header(&temperatures, &measurement, true)?;
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
        .set(0, &(0..8).map(|i| i as f32 / 7.0).collect::<Vec<_>>(), true)?;
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
        .set(String::from("ValueTake payload from server"), false, true)?;
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
        .set(0, &[0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0], false, true, true)?;
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

    states
        .image
        .images
        .set(2, &image, [256, 256], s::ImageFormat::Color, true)?;
    states
        .image
        .images
        .set_all(7, [256, 256], s::ImageColor::Color(25, 35, 75), true)?;
    states.image.images.update(
        7,
        &image_patch,
        [96, 96],
        [64, 64],
        s::ImageFormat::ColorAlpha,
        true,
        false,
    )?;

    Ok(())
}
