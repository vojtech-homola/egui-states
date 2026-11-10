use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::protocol::image::{ImageType, checked_image_bytes};
use crate::server_core::states::image::single::ImageData;

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

/// Whether a buffer is indirect: a sub-offset of zero or more means the
/// values along that dimension are pointers, so `buf_ptr()` does not point at
/// pixel rows.
fn is_indirect(suboffsets: Option<&[isize]>) -> bool {
    suboffsets.is_some_and(|offsets| offsets.iter().any(|&offset| offset >= 0))
}

fn check_direct(image: &PyBuffer<u8>) -> PyResult<()> {
    if is_indirect(image.suboffsets()) {
        return Err(PyValueError::new_err(
            "Indirect buffers with sub-offsets are not supported",
        ));
    }
    Ok(())
}

pub(crate) fn image_data(image: &PyBuffer<u8>) -> PyResult<ImageData<'_>> {
    check_direct(image)?;
    let shape = image.shape();
    let strides = image.strides();
    let image_type = check_image_type(shape, strides)?;
    let size = [shape[0], shape[1]];
    checked_image_bytes(size, 4).map_err(PyValueError::new_err)?;
    if size[0] == 0 || size[1] == 0 {
        return Err(PyValueError::new_err("Image dimensions cannot be zero"));
    }

    let row_bytes = size[1] * image_type.bytes_per_pixel();
    let stride = if image.is_c_contiguous() {
        row_bytes
    } else {
        if strides[0] <= 0 {
            return Err(PyValueError::new_err("Invalid strides"));
        }
        strides[0] as usize
    };
    let span = (size[0] - 1)
        .checked_mul(stride)
        .and_then(|rows| rows.checked_add(row_bytes))
        .ok_or_else(|| PyValueError::new_err("Invalid strides"))?;
    // SAFETY: a direct buffer with this shape and these strides holds `span`
    // readable bytes from `buf_ptr()`, and `image` keeps them alive while the
    // view is borrowed.
    let data = unsafe { std::slice::from_raw_parts(image.buf_ptr().cast::<u8>(), span) };

    Ok(ImageData {
        size,
        stride,
        image_type,
        data,
    })
}

pub(crate) fn image_color(color: &Bound<'_, PyAny>) -> PyResult<[u8; 4]> {
    if let Ok(gray) = color.extract::<u8>() {
        return Ok([gray, gray, gray, 255]);
    }
    if let Ok((gray, alpha)) = color.extract::<(u8, u8)>() {
        return Ok([gray, gray, gray, alpha]);
    }
    if let Ok((red, green, blue)) = color.extract::<(u8, u8, u8)>() {
        return Ok([red, green, blue, 255]);
    }
    if let Ok((red, green, blue, alpha)) = color.extract::<(u8, u8, u8, u8)>() {
        return Ok([red, green, blue, alpha]);
    }

    Err(PyValueError::new_err(
        "color must be an integer or a tuple of 2, 3, or 4 integers in the range 0..=255",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_negative_sub_offsets_make_a_buffer_indirect() {
        assert!(!is_indirect(None));
        assert!(!is_indirect(Some(&[-1, -1, -1])));
        assert!(is_indirect(Some(&[0, -1])));
        assert!(is_indirect(Some(&[-1, 16])));
    }
}
