use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use egui_states_core::image::ImageType;

use crate::image::{ImageData, ValueImage};

fn check_image_type(shape: &[usize], strides: &[isize]) -> PyResult<ImageType> {
    match shape.len() {
        2 => {
            if strides[1] == 1 {
                return Ok(ImageType::Gray);
            }
            Err(PyValueError::new_err("Invalid strides"))
        }
        3 => {
            if strides[2] != 1 {
                return Err(PyValueError::new_err("Invalid strides"));
            }
            match shape[2] {
                2 => {
                    if strides[1] != 2 {
                        return Err(PyValueError::new_err("Invalid strides"));
                    }
                    Ok(ImageType::GrayAlpha)
                }
                3 => {
                    if strides[1] != 3 {
                        return Err(PyValueError::new_err("Invalid strides"));
                    }

                    Ok(ImageType::Color)
                }
                4 => {
                    if strides[1] != 4 {
                        return Err(PyValueError::new_err("Invalid strides"));
                    }
                    Ok(ImageType::ColorAlpha)
                }
                _ => Err(PyValueError::new_err("Invalid image dimensions")),
            }
        }
        _ => Err(PyValueError::new_err("Invalid image dimensions")),
    }
}

pub(crate) fn set_image(
    image: &PyBuffer<u8>,
    image_value: &ValueImage,
    origin: Option<[u32; 2]>,
    update: bool,
) -> PyResult<()> {
    let shape = image.shape();
    let strides = image.strides();
    let contiguous = image.is_c_contiguous();
    let image_type = check_image_type(shape, strides)?;
    let size = [shape[0], shape[1]];

    // get data stride
    let stride = if contiguous {
        0 // do not use strides
    } else {
        if strides[0] <= 0 {
            return Err(PyValueError::new_err("Invalid strides"));
        }
        strides[0] as usize
    };

    let data = image.buf_ptr() as *const u8;

    let image_data = ImageData {
        size,
        stride,
        contiguous,
        image_type,
        data,
    };

    image_value
        .set_image(image_data, origin, update)
        .map_err(|e| PyValueError::new_err(format!("Failed to set image: {}", e)))
}
