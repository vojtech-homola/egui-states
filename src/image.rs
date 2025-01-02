use std::ptr::copy_nonoverlapping;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use egui::{Color32, ColorImage, ImageData, TextureHandle};
use postcard;
use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyByteArray;
use serde::{Deserialize, Serialize};

use crate::transport::{serialize, WriteMessage};
use crate::SyncTrait;

#[derive(Clone, Copy, Serialize, Deserialize)]
enum ImageType {
    Color,
    ColorAlpha,
    Gray,
    GrayAlpha,
}

#[derive(Serialize, Deserialize)]
struct ImageInfo {
    pub image_size: [usize; 2],   // [y, x]
    pub rect: Option<[usize; 4]>, // [y, x, h, w]
    pub image_type: ImageType,
}

pub(crate) trait ImageUpdate: Send + Sync {
    fn update_image(&self, data: &[u8]) -> Result<(), String>;
}

const TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Nearest,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

pub struct ValueImage {
    id: u32,
    texture_handle: RwLock<(Option<TextureHandle>, [usize; 2])>,
}

impl ValueImage {
    pub fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            texture_handle: RwLock::new((None, [0, 0])),
        })
    }

    pub fn get_id(&self) -> egui::TextureId {
        self.texture_handle.read().unwrap().0.as_ref().unwrap().id()
    }

    pub fn get_size(&self) -> [usize; 2] {
        self.texture_handle.read().unwrap().1
    }

    pub fn initialize(&self, ctx: &egui::Context) {
        const SIZE: usize = 512;
        let mut color_image = ColorImage::new([SIZE, SIZE], Color32::BLACK);
        for i in 0..SIZE {
            for j in 0..SIZE {
                let pixel = (i + j) as u8;
                color_image.pixels[i * SIZE + j] = egui::Color32::from_gray(pixel);
            }
        }

        let image_data = ImageData::Color(Arc::new(color_image));
        let name = format!("image_{}", self.id);
        let texture_handle = ctx.load_texture(name, image_data, TEXTURE_OPTIONS);

        let mut w = self.texture_handle.write().unwrap();
        let size = texture_handle.size();
        match *w {
            (None, _) => {
                w.0 = Some(texture_handle);
                w.1 = size;
            }
            _ => {}
        }
    }
}

impl ImageUpdate for ValueImage {
    fn update_image(&self, data: &[u8]) -> Result<(), String> {
        let (info, image_data) = postcard::take_from_bytes(data).map_err(|e| {
            format!(
                "Failed to deserialize image message: {} for image of id {}",
                e, self.id
            )
        })?;

        let ImageInfo {
            image_size,
            rect,
            image_type,
        } = info;

        let size = match rect {
            Some(r) => {
                if r[0] + r[2] > image_size[0] || r[1] + r[3] > image_size[1] {
                    return Err("Rectangle is out of bounds".to_string());
                }
                [r[3], r[2]]
            }
            None => [image_size[1], image_size[0]],
        };

        // TODO: cache the color image
        let mut c_image = egui::ColorImage::new(size, egui::Color32::WHITE);
        let pixel_count = size[0] * size[1];

        let data_ptr = image_data.as_ptr();
        let image_ptr = c_image.pixels.as_mut_ptr() as *mut u8;

        match image_type {
            ImageType::Color => {
                for i in 0..pixel_count {
                    let idx = i * 3;
                    let im_idx = i * 4;
                    unsafe {
                        *image_ptr.add(im_idx) = *data_ptr.add(idx);
                        *image_ptr.add(im_idx + 1) = *data_ptr.add(idx + 1);
                        *image_ptr.add(im_idx + 2) = *data_ptr.add(idx + 2);
                        *image_ptr.add(im_idx + 3) = 255;
                    }
                }
            }

            ImageType::ColorAlpha => unsafe {
                copy_nonoverlapping(data_ptr, image_ptr, pixel_count * 4);
            },

            ImageType::Gray => {
                for i in 0..pixel_count {
                    let im_idx = i * 4;
                    unsafe {
                        let pixel = *data_ptr.add(i);
                        *image_ptr.add(im_idx) = pixel;
                        *image_ptr.add(im_idx + 1) = pixel;
                        *image_ptr.add(im_idx + 2) = pixel;
                        *image_ptr.add(im_idx + 3) = 255;
                    }
                }
            }

            ImageType::GrayAlpha => {
                for i in 0..pixel_count {
                    let im_idx = i * 4;
                    unsafe {
                        let pixel = *data_ptr.add(i * 2);
                        *image_ptr.add(im_idx) = pixel;
                        *image_ptr.add(im_idx + 1) = pixel;
                        *image_ptr.add(im_idx + 2) = pixel;
                        *image_ptr.add(im_idx + 3) = *data_ptr.add(i * 2 + 1);
                    }
                }
            }
        }

        let mut w = self.texture_handle.write().unwrap();
        let previous_size = w.1;
        if let Some(ref mut texture_handle) = w.0 {
            match rect {
                Some(rec) => {
                    if previous_size[0] != image_size[1] || previous_size[1] != image_size[0] {
                        return Err(
                            "Rectangle is set but the image size is different from texture"
                                .to_string(),
                        );
                    }
                    texture_handle.set_partial([rec[1], rec[0]], c_image, TEXTURE_OPTIONS);
                }
                None => {
                    texture_handle.set(c_image, TEXTURE_OPTIONS);
                    w.1 = size;
                }
            }
        }

        Ok(())
    }
}

// SERVER -----------------------------------------------------
// ------------------------------------------------------------
struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
}

pub(crate) struct PyValueImage {
    id: u32,
    image: RwLock<ImageDataInner>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl PyValueImage {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
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

    pub(crate) fn get_size_py(&self) -> [usize; 2] {
        self.image.read().unwrap().size
    }

    pub(crate) fn get_image_py<'py>(
        &self,
        py: Python<'py>,
    ) -> (Bound<'py, PyByteArray>, [usize; 2]) {
        let w = self.image.read().unwrap();
        let size = w.size;
        let data = PyByteArray::new(py, &w.data);
        (data, size)
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
        let data = if self.connected.load(Ordering::Relaxed) {
            let data_size = image.item_count();
            let mut data = Vec::with_capacity(data_size);
            if contiguous {
                let buffer = image.buf_ptr() as *const u8;
                unsafe {
                    std::ptr::copy_nonoverlapping(buffer, data.as_mut_ptr(), data_size);
                }
            } else {
                let image_ptr = image.buf_ptr() as *const u8;
                let data_ptr = data.as_mut_ptr();
                let line_size = size[1] * strides[1] as usize;
                for i in 0..size[0] {
                    let buffer = unsafe { image_ptr.add(i * stride) };
                    let data_buffer = unsafe { data_ptr.add(i * line_size) };
                    unsafe { std::ptr::copy_nonoverlapping(buffer, data_buffer, line_size) };
                }
                contiguous = true;
                stride = 0;
            }
            unsafe { data.set_len(data_size) };
            data_ptr = data.as_ptr();
            Some(data)
        } else {
            data_ptr = image.buf_ptr() as *const u8;
            None
        };

        // write data to the image
        let mut w = self.image.write().unwrap();
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
        let new_size = w.size;

        // send the image to the server
        if let Some(data) = data {
            let rect = origin.map(|o| [o[0], o[1], size[0], size[1]]);
            let image_info = ImageInfo {
                image_size: new_size,
                rect,
                image_type,
            };
            let info = serialize(&image_info);
            let message = WriteMessage::Image(self.id, update, info, data);
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

        let image_info = ImageInfo {
            image_size: w.size,
            rect: None,
            image_type: ImageType::ColorAlpha,
        };
        let info = serialize(&image_info);
        let image_data = w.data.clone();
        drop(w);

        let message = WriteMessage::Image(self.id, false, info, image_data);
        self.channel.send(message).unwrap();
    }
}

fn check_image_type(shape: &[usize], strides: &[isize]) -> PyResult<ImageType> {
    match shape.len() {
        2 => {
            if strides[1] != 1 {
                return Err(PyValueError::new_err("Invalid strides"));
            }
            Ok(ImageType::Gray)
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
        ImageType::ColorAlpha => {
            std::ptr::copy_nonoverlapping(data, new_data, all_size * 4);
        }
        ImageType::Color => {
            for i in 0..all_size {
                *new_data.add(i * 4) = *data.add(i * 3);
                *new_data.add(i * 4 + 1) = *data.add(i * 3 + 1);
                *new_data.add(i * 4 + 2) = *data.add(i * 3 + 2);
                *new_data.add(i * 4 + 3) = 255;
            }
        }
        ImageType::Gray => {
            for i in 0..all_size {
                let p = *data.add(i);
                *new_data.add(i * 4) = p;
                *new_data.add(i * 4 + 1) = p;
                *new_data.add(i * 4 + 2) = p;
                *new_data.add(i * 4 + 3) = 255;
            }
        }

        ImageType::GrayAlpha => {
            for i in 0..all_size {
                let p = *data.add(i * 2);
                *new_data.add(i * 4) = p;
                *new_data.add(i * 4 + 1) = p;
                *new_data.add(i * 4 + 2) = p;
                *new_data.add(i * 4 + 3) = *data.add(i * 2 + 1);
            }
        }
    }
    new_data_vec.set_len(all_size * 4);
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
        ImageType::ColorAlpha => {
            for i in 0..size[0] {
                let buffer = data.add(i * stride);
                let data_buffer = new_data.add(i * size[1] * 4);
                std::ptr::copy_nonoverlapping(buffer, data_buffer, size[1] * 4);
            }
        }
        ImageType::Color => {
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
        }
        ImageType::Gray => {
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
        }
        ImageType::GrayAlpha => {
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
        }
    }
    new_data_vec.set_len(all_size * 4);
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
        ImageType::ColorAlpha => {
            if stride == 0 {
                stride = size[1] * 4;
            }
            for i in 0..size[0] {
                for j in 0..size[1] {
                    let index = (top + i) * old_stride + left + j;
                    let d_index = i * stride + j * 4;
                    *old_data.add(index * 4) = *data.add(d_index);
                    *old_data.add(index * 4 + 1) = *data.add(d_index + 1);
                    *old_data.add(index * 4 + 2) = *data.add(d_index + 2);
                    *old_data.add(index * 4 + 3) = *data.add(d_index + 3);
                }
            }
        }
        ImageType::Color => {
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
        }
        ImageType::Gray => {
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
        }
        ImageType::GrayAlpha => {
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
        }
    }
}
