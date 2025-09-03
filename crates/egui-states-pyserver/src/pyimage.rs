use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyByteArray;
use tungstenite::Bytes;

use egui_states_core::image::{ImageInfo, ImageType};

use crate::server::SyncTrait;

struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
}

pub(crate) struct PyValueImage {
    id: u32,
    image: RwLock<ImageDataInner>,
    channel: Sender<Option<Bytes>>,
    connected: Arc<AtomicBool>,
}

impl PyValueImage {
    pub(crate) fn new(
        id: u32,
        channel: Sender<Option<Bytes>>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            image: RwLock::new(ImageDataInner {
                data: Vec::with_capacity(0),
                size: [0, 0],
            }),
            channel,
            connected,
        })
    }

    pub(crate) fn get_size_py(&self) -> (usize, usize) {
        let size = self.image.read().unwrap().size;
        (size[0], size[1])
    }

    pub(crate) fn get_image_py<'py>(
        &self,
        py: Python<'py>,
    ) -> (Bound<'py, PyByteArray>, (usize, usize)) {
        let w = self.image.read().unwrap();
        let size = w.size;
        let data = PyByteArray::new(py, &w.data);
        (data, (size[0], size[1]))
    }

    // Function is complex because it needs to handle different image types and also not contiguous
    // data. Also it tries to avoid copying data if possible.
    pub(crate) fn set_image_py(
        &self,
        image: &PyBuffer<u8>,
        origin: Option<[usize; 2]>,
        update: bool,
    ) -> PyResult<()> {
        let shape = image.shape();
        let strides = image.strides();
        let mut contiguous = image.is_c_contiguous();
        let image_type = check_image_type(shape, strides)?;
        let size = [shape[0], shape[1]];

        // get data stride
        let mut stride = if contiguous {
            0 // do not use strides
        } else {
            if strides[0] <= 0 {
                return Err(PyValueError::new_err("Invalid strides"));
            }
            strides[0] as usize
        };

        // get data pointer and prepare data
        let data_ptr;
        let mut w = self.image.write().unwrap();
        let data = if self.connected.load(Ordering::Relaxed) {
            let new_size = match origin {
                Some(_) => w.size, // keep the old size
                None => size,      // use the new size
            };

            let message = ImageInfo {
                image_size: new_size,
                rect: origin.map(|o| [o[0], o[1], size[0], size[1]]),
                image_type,
                update,
            };
            let mut head_buff = [0u8; 64];
            let buff = message.serialize(self.id, &mut head_buff);
            let offset = buff.len();
            let data_size = image.item_count();

            let mut data = Vec::with_capacity(data_size + offset);
            unsafe { data.set_len(data_size + offset) };
            data[..offset].copy_from_slice(&head_buff);

            if contiguous {
                let buffer = image.buf_ptr() as *const u8;
                let data_ptr = unsafe { data.as_mut_ptr().add(offset) };
                unsafe {
                    std::ptr::copy_nonoverlapping(buffer, data_ptr, data_size);
                }
            } else {
                let image_ptr = image.buf_ptr() as *const u8;
                let data_ptr = unsafe { data.as_mut_ptr().add(offset) };
                let line_size = size[1] * strides[1] as usize;
                for i in 0..size[0] {
                    let buffer = unsafe { image_ptr.add(i * stride) };
                    let data_buffer = unsafe { data_ptr.add(i * line_size) };
                    unsafe { std::ptr::copy_nonoverlapping(buffer, data_buffer, line_size) };
                }
                contiguous = true;
                stride = 0;
            }

            data_ptr = data.as_ptr();
            Some(data)
        } else {
            data_ptr = image.buf_ptr() as *const u8;
            None
        };

        // write data to the image
        match origin {
            Some(origin) => {
                let original_size = w.size;

                // check if the rectangle fits in the original image
                if origin[0] + size[0] > original_size[0] || origin[1] + size[1] > original_size[1]
                {
                    return Err(PyValueError::new_err(format!(
                        "rectangle {:?} does not fit in the original image with size {:?}",
                        origin, original_size
                    )));
                }

                let old_data_ptr = w.data.as_mut_ptr();
                unsafe {
                    write_rectangle(
                        data_ptr,
                        stride,
                        old_data_ptr,
                        original_size[1],
                        &origin,
                        &size,
                        image_type,
                    );
                }
            }
            None => {
                if contiguous {
                    w.data = unsafe { write_all_new(data_ptr, &size, image_type) };
                } else {
                    w.data = unsafe { write_all_new_stride(data_ptr, stride, &size, image_type) };
                }
                w.size = size;
            }
        }

        // send the image to the server
        if let Some(data) = data {
            let message = Some(Bytes::from(data));
            self.channel.send(message).unwrap();
        }

        Ok(())
    }
}

impl SyncTrait for PyValueImage {
    fn sync(&self) {
        let w = self.image.read().unwrap();
        if w.size[0] == 0 || w.size[1] == 0 {
            return;
        }

        let mut head_buff = [0u8; 32];
        let image_info = ImageInfo {
            image_size: w.size,
            rect: None,
            image_type: ImageType::ColorAlpha,
            update: false,
        };
        let buff = image_info.serialize(self.id, &mut head_buff);

        let mut data = Vec::with_capacity(buff.len() + w.data.len());
        unsafe { data.set_len(buff.len() + w.data.len()) };
        data[..buff.len()].copy_from_slice(buff);
        data[buff.len()..].copy_from_slice(&w.data);
        drop(w);

        let message = Some(Bytes::from(data));
        self.channel.send(message).unwrap();
    }
}

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

unsafe fn write_all_new(data: *const u8, size: &[usize; 2], image_type: ImageType) -> Vec<u8> {
    let all_size = size[0] * size[1];
    let mut new_data_vec: Vec<u8> = Vec::with_capacity(all_size * 4);
    let new_data = new_data_vec.as_mut_ptr();

    match image_type {
        ImageType::ColorAlpha => unsafe {
            std::ptr::copy_nonoverlapping(data, new_data, all_size * 4);
        },
        ImageType::Color => unsafe {
            for i in 0..all_size {
                *new_data.add(i * 4) = *data.add(i * 3);
                *new_data.add(i * 4 + 1) = *data.add(i * 3 + 1);
                *new_data.add(i * 4 + 2) = *data.add(i * 3 + 2);
                *new_data.add(i * 4 + 3) = 255;
            }
        },
        ImageType::Gray => unsafe {
            for i in 0..all_size {
                let p = *data.add(i);
                *new_data.add(i * 4) = p;
                *new_data.add(i * 4 + 1) = p;
                *new_data.add(i * 4 + 2) = p;
                *new_data.add(i * 4 + 3) = 255;
            }
        },

        ImageType::GrayAlpha => unsafe {
            for i in 0..all_size {
                let p = *data.add(i * 2);
                *new_data.add(i * 4) = p;
                *new_data.add(i * 4 + 1) = p;
                *new_data.add(i * 4 + 2) = p;
                *new_data.add(i * 4 + 3) = *data.add(i * 2 + 1);
            }
        },
    }
    unsafe { new_data_vec.set_len(all_size * 4) };
    new_data_vec
}

unsafe fn write_all_new_stride(
    data: *const u8,
    stride: usize,
    size: &[usize; 2],
    image_type: ImageType,
) -> Vec<u8> {
    let all_size = size[0] * size[1];
    let mut new_data_vec: Vec<u8> = Vec::with_capacity(all_size * 4);
    let new_data = new_data_vec.as_mut_ptr();

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
    unsafe { new_data_vec.set_len(all_size * 4) };
    new_data_vec
}

unsafe fn write_rectangle(
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
                // for j in 0..size[1] {
                //     let index = (top + i) * old_stride + left + j;
                //     let d_index = i * stride + j * 4;
                //     *old_data.add(index * 4) = *data.add(d_index);
                //     *old_data.add(index * 4 + 1) = *data.add(d_index + 1);
                //     *old_data.add(index * 4 + 2) = *data.add(d_index + 2);
                //     *old_data.add(index * 4 + 3) = *data.add(d_index + 3);
                // }
                let index = (top + i) * old_stride + left * 4;
                let buffer = data.add(i * stride);
                let data_buffer = old_data.add(index);
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
