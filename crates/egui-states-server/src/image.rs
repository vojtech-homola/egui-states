use egui_states_core::serialization::ServerHeader;
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::image::{ImageHeader, ImageType};

use crate::event::Event;
use crate::sender::MessageSender;
use crate::server::{Acknowledge, EnableTrait, SyncTrait};

struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
}

pub(crate) struct ImageData {
    pub size: [usize; 2],
    pub stride: usize,
    pub contiguous: bool,
    pub image_type: ImageType,
    pub data: *const u8,
}

pub(crate) struct ValueImage {
    id: u64,
    image: RwLock<ImageDataInner>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    enabled: AtomicBool,
    event: Event,
}

impl ValueImage {
    pub(crate) fn new(id: u64, sender: MessageSender, connected: Arc<AtomicBool>) -> Arc<Self> {
        let event = Event::new();
        event.set(); // initially set so the first send does not block

        Arc::new(Self {
            id,
            image: RwLock::new(ImageDataInner {
                data: Vec::with_capacity(0),
                size: [0, 0],
            }),
            sender,
            connected,
            enabled: AtomicBool::new(false),
            event,
        })
    }

    pub(crate) fn get_size(&self) -> [usize; 2] {
        self.image.read().size
    }

    pub(crate) fn get_image<T>(&self, getter: impl FnOnce((&Vec<u8>, &[usize; 2])) -> T) -> T {
        let w = self.image.read();
        getter((&w.data, &w.size))
    }

    // Function is complex because it needs to handle different image types and also not contiguous
    // data. Also it tries to avoid copying data if possible.
    pub(crate) fn set_image(
        &self,
        image: ImageData,
        origin: Option<[u32; 2]>,
        update: bool,
    ) -> Result<(), String> {
        let mut stride = image.stride;
        let mut contiguous = image.contiguous;

        // get data pointer and prepare data
        let data_ptr;
        let mut w = self.image.write();
        let data = if self.connected.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed)
        {
            let new_size = match origin {
                Some(_) => w.size,  // keep the old size
                None => image.size, // use the new size
            };

            let image_header = ImageHeader {
                image_size: [new_size[0] as u32, new_size[1] as u32],
                rect: origin.map(|o| [o[0], o[1], image.size[0] as u32, image.size[1] as u32]),
                image_type: image.image_type,
            };
            let mut head_buff = [0u8; 64];
            let header = ServerHeader::Image(self.id, update, image_header);
            let buff = header.serialize_to_slice(&mut head_buff);

            let offset = buff.len();
            let data_size = image.size[0] * image.size[1] * image.image_type.bytes_per_pixel();

            let mut data = Vec::with_capacity(data_size + offset);
            unsafe { data.set_len(data_size + offset) };
            data[..offset].copy_from_slice(&buff);

            if contiguous {
                let buffer = image.data;
                let data_ptr = unsafe { data.as_mut_ptr().add(offset) };
                unsafe {
                    std::ptr::copy_nonoverlapping(buffer, data_ptr, data_size);
                }
            } else {
                let image_ptr = image.data;
                let data_ptr = unsafe { data.as_mut_ptr().add(offset) };
                let line_size = image.size[1] * image.image_type.bytes_per_pixel();
                for i in 0..image.size[0] {
                    let buffer = unsafe { image_ptr.add(i * stride) };
                    let data_buffer = unsafe { data_ptr.add(i * line_size) };
                    unsafe { std::ptr::copy_nonoverlapping(buffer, data_buffer, line_size) };
                }
                contiguous = true;
                stride = 0;
            }

            data_ptr = unsafe { data.as_ptr().add(offset) };
            Some(data)
        } else {
            data_ptr = image.data;
            None
        };

        // write data to the image
        match origin {
            Some(origin) => {
                let origin = [origin[0] as usize, origin[1] as usize];
                let original_size = w.size;

                // check if the rectangle fits in the original image
                if origin[0] + image.size[0] > original_size[0]
                    || origin[1] + image.size[1] > original_size[1]
                {
                    return Err(format!(
                        "rectangle {:?} does not fit in the original image with size {:?}",
                        origin, original_size
                    ));
                }

                let old_data_ptr = w.data.as_mut_ptr();
                unsafe {
                    write_rectangle(
                        data_ptr,
                        stride,
                        old_data_ptr,
                        original_size[1],
                        &origin,
                        &image.size,
                        image.image_type,
                    );
                }
            }
            None => {
                if contiguous {
                    w.data = unsafe { write_all_new(data_ptr, &image.size, image.image_type) };
                } else {
                    w.data = unsafe {
                        write_all_new_stride(data_ptr, stride, &image.size, image.image_type)
                    };
                }
                w.size = image.size;
            }
        }

        self.event.wait_lock();
        if !self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }
        // send the image to the server
        if let Some(data) = data {
            self.sender.send(Bytes::from(data));
        }

        Ok(())
    }
}

impl Acknowledge for ValueImage {
    fn acknowledge(&self) {
        self.event.set();
    }
}

impl EnableTrait for ValueImage {
    fn enable(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Relaxed);
    }
}

impl SyncTrait for ValueImage {
    fn sync(&self) {
        let w = self.image.read();
        if !self.enabled.load(Ordering::Relaxed) || w.size[0] == 0 || w.size[1] == 0 {
            self.event.set();
            return;
        }

        let mut head_buff = [0u8; 64];
        let image_header = ImageHeader {
            image_size: [w.size[0] as u32, w.size[1] as u32],
            rect: None,
            image_type: ImageType::ColorAlpha,
        };
        let header = ServerHeader::Image(self.id, false, image_header);
        let buff = header.serialize_to_slice(&mut head_buff);

        let mut data = Vec::with_capacity(buff.len() + w.data.len());
        unsafe { data.set_len(buff.len() + w.data.len()) };
        data[..buff.len()].copy_from_slice(buff);
        data[buff.len()..].copy_from_slice(&w.data);
        drop(w);

        self.event.clear();
        self.sender.send(Bytes::from(data));
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
