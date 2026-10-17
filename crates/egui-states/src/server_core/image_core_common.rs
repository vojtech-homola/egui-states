use crate::image_transport::{
    ImageType, checked_image_bounds, checked_image_bytes, checked_image_shape,
};

pub(crate) fn checked_image_size(size: [usize; 2]) -> Result<[u32; 2], String> {
    let [height, width] = checked_image_shape(size)?;
    if size[0] == 0 || size[1] == 0 {
        return Err("Image dimensions cannot be zero".to_string());
    }

    Ok([width, height])
}

pub(crate) fn checked_image_rect(
    origin: &[usize; 2],
    size: [usize; 2],
) -> Result<[u32; 4], String> {
    let [width, height] = checked_image_size(size)?;
    checked_image_bounds(*origin, size)?;
    let [y, x] = checked_image_shape(*origin)?;

    Ok([x, y, width, height])
}

pub(crate) fn checked_fill_size(size: [usize; 2]) -> Result<([u32; 2], usize), String> {
    let wire_size = checked_image_size(size)?;
    let rgba_size = checked_image_bytes(size, 4)?;

    Ok((wire_size, rgba_size))
}

#[inline]
pub(super) unsafe fn write_all_new(
    data: *const u8,
    new_data: *mut u8,
    pixel_count: usize,
    image_type: ImageType,
) {
    match image_type {
        ImageType::ColorAlpha => unsafe {
            std::ptr::copy_nonoverlapping(data, new_data, pixel_count * 4);
        },
        ImageType::Color => unsafe {
            for i in 0..pixel_count {
                *new_data.add(i * 4) = *data.add(i * 3);
                *new_data.add(i * 4 + 1) = *data.add(i * 3 + 1);
                *new_data.add(i * 4 + 2) = *data.add(i * 3 + 2);
                *new_data.add(i * 4 + 3) = 255;
            }
        },
        ImageType::Gray => unsafe {
            for i in 0..pixel_count {
                let p = *data.add(i);
                *new_data.add(i * 4) = p;
                *new_data.add(i * 4 + 1) = p;
                *new_data.add(i * 4 + 2) = p;
                *new_data.add(i * 4 + 3) = 255;
            }
        },

        ImageType::GrayAlpha => unsafe {
            for i in 0..pixel_count {
                let p = *data.add(i * 2);
                *new_data.add(i * 4) = p;
                *new_data.add(i * 4 + 1) = p;
                *new_data.add(i * 4 + 2) = p;
                *new_data.add(i * 4 + 3) = *data.add(i * 2 + 1);
            }
        },
    }
}

#[inline]
pub(super) unsafe fn write_all_new_stride(
    data: *const u8,
    new_data: *mut u8,
    stride: usize,
    size: &[usize; 2],
    image_type: ImageType,
) {
    match image_type {
        ImageType::ColorAlpha => unsafe {
            for i in 0..size[0] {
                let buffer = data.add(i * stride);
                let data_buffer = new_data.add(i * size[1] * 4);
                std::ptr::copy_nonoverlapping(buffer, data_buffer, size[1] * 4);
            }
        },
        ImageType::Color => unsafe {
            for i in 0..size[0] {
                let buffer = data.add(i * stride);
                let data_buffer = new_data.add(i * size[1] * 4);
                for j in 0..size[1] {
                    *data_buffer.add(j * 4) = *buffer.add(j * 3);
                    *data_buffer.add(j * 4 + 1) = *buffer.add(j * 3 + 1);
                    *data_buffer.add(j * 4 + 2) = *buffer.add(j * 3 + 2);
                    *data_buffer.add(j * 4 + 3) = 255;
                }
            }
        },
        ImageType::Gray => unsafe {
            for i in 0..size[0] {
                let buffer = data.add(i * stride);
                let data_buffer = new_data.add(i * size[1] * 4);
                for j in 0..size[1] {
                    let p = *buffer.add(j);
                    *data_buffer.add(j * 4) = p;
                    *data_buffer.add(j * 4 + 1) = p;
                    *data_buffer.add(j * 4 + 2) = p;
                    *data_buffer.add(j * 4 + 3) = 255;
                }
            }
        },
        ImageType::GrayAlpha => unsafe {
            for i in 0..size[0] {
                let buffer = data.add(i * stride);
                let data_buffer = new_data.add(i * size[1] * 4);
                for j in 0..size[1] {
                    let p = *buffer.add(j * 2);
                    *data_buffer.add(j * 4) = p;
                    *data_buffer.add(j * 4 + 1) = p;
                    *data_buffer.add(j * 4 + 2) = p;
                    *data_buffer.add(j * 4 + 3) = *buffer.add(j * 2 + 1);
                }
            }
        },
    }
}

#[inline]
pub(super) unsafe fn write_rectangle(
    data: *const u8,
    mut stride: usize,
    old_data: *mut u8,
    old_stride: usize,
    origin: &[usize; 2],
    size: &[usize; 2],
    image_type: ImageType,
) {
    let top = origin[0];
    let left = origin[1];

    match image_type {
        ImageType::ColorAlpha => unsafe {
            if stride == 0 {
                stride = size[1] * 4;
            }
            let x = size[1] * 4;
            for i in 0..size[0] {
                let index = (top + i) * old_stride + left;
                let buffer = data.add(i * stride);
                let data_buffer = old_data.add(index * 4);
                std::ptr::copy_nonoverlapping(buffer, data_buffer, x);
            }
        },
        ImageType::Color => unsafe {
            if stride == 0 {
                stride = size[1] * 3;
            }
            for i in 0..size[0] {
                for j in 0..size[1] {
                    let index = (top + i) * old_stride + left + j;
                    let d_index = i * stride + j * 3;
                    *old_data.add(index * 4) = *data.add(d_index);
                    *old_data.add(index * 4 + 1) = *data.add(d_index + 1);
                    *old_data.add(index * 4 + 2) = *data.add(d_index + 2);
                    *old_data.add(index * 4 + 3) = 255;
                }
            }
        },
        ImageType::Gray => unsafe {
            if stride == 0 {
                stride = size[1];
            }
            for i in 0..size[0] {
                for j in 0..size[1] {
                    let index = (top + i) * old_stride + left + j;
                    let p = *data.add(i * stride + j);
                    *old_data.add(index * 4) = p;
                    *old_data.add(index * 4 + 1) = p;
                    *old_data.add(index * 4 + 2) = p;
                    *old_data.add(index * 4 + 3) = 255;
                }
            }
        },
        ImageType::GrayAlpha => unsafe {
            if stride == 0 {
                stride = size[1] * 2;
            }
            for i in 0..size[0] {
                for j in 0..size[1] {
                    let index = (top + i) * old_stride + left + j;
                    let d_index = i * stride + j * 2;
                    let p = *data.add(d_index);
                    *old_data.add(index * 4) = p;
                    *old_data.add(index * 4 + 1) = p;
                    *old_data.add(index * 4 + 2) = p;
                    *old_data.add(index * 4 + 3) = *data.add(d_index + 1);
                }
            }
        },
    }
}
