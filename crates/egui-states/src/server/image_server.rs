use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::image_transport::{ImageHeader, ImageSetHeader, ImageType};
use crate::serialization::ServerHeader;
use crate::serialization::{FastVec, MSG_SIZE_THRESHOLD};
use crate::server::event::Event;
use crate::server::sender::{MessageSender, SenderData};
use crate::server::server::{Acknowledge, SyncTrait};

struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
    buffer: Option<([usize; 4], VecDeque<(FastVec<32>, bool)>)>,
}

pub(crate) struct ImageData {
    pub size: [usize; 2],
    pub stride: usize,
    pub contiguous: bool,
    pub image_type: ImageType,
    pub data: *const u8,
}

enum SetData {
    Single(FastVec<32>),
    Multi(Vec<(FastVec<32>, bool)>),
}

enum SetDataUpdate {
    Single(FastVec<32>),
    Multi(VecDeque<(FastVec<32>, bool)>),
}

pub(crate) struct ValueImage {
    pub(crate) name: String,
    id: u64,
    image: RwLock<ImageDataInner>,
    lock: Mutex<()>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl ValueImage {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let event = Event::new();
        event.set(); // initially set so the first send does not block

        Arc::new(Self {
            name,
            id,
            image: RwLock::new(ImageDataInner {
                data: Vec::with_capacity(0),
                size: [0, 0],
                update_rect: [0, 0, 0, 0],
                buffer: VecDeque::new(),
            }),
            lock: Mutex::new(()),
            sender,
            connected,
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

    pub(crate) fn set_image(&self, image: ImageData, update: bool) -> Result<(), String> {
        // Prepare data to send if connected
        let to_send = if self.connected.load(Ordering::Relaxed) {
            Some(pack_set_data(self.id, &image, update)?)
        } else {
            None
        };

        let pixels_count = image.size[0] * image.size[1];

        // this is main lock for set and update operation
        let _lock = self.lock.lock();
        let mut w = self.image.write();

        // only allocate new data if size has changed
        if image.size != w.size {
            w.size = image.size;
            let mut new_data = Vec::with_capacity(pixels_count * 4);
            unsafe {
                new_data.set_len(pixels_count * 4);
            }
            w.data = new_data;
        }

        if image.contiguous {
            unsafe {
                write_all_new(
                    image.data,
                    w.data.as_mut_ptr(),
                    pixels_count,
                    image.image_type,
                )
            };
        } else {
            unsafe {
                write_all_new_stride(
                    image.data,
                    w.data.as_mut_ptr(),
                    image.stride,
                    &image.size,
                    image.image_type,
                )
            };
        }

        w.buffer.clear();
        drop(w);

        if let Some(to_send) = to_send {
            self.event.wait_clear();
            if !self.connected.load(Ordering::Relaxed) {
                return Ok(());
            }

            match to_send {
                SetData::Single(data) => {
                    self.sender.send_single(data);
                }
                SetData::Multi(data) => {
                    for (message, send_now) in data {
                        self.sender.send_set(message, send_now);
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn update_image(
        &self,
        origin: &[usize; 2],
        image: ImageData,
        update: bool,
        force: bool,
    ) -> Result<(), String> {
        let to_send = if self.connected.load(Ordering::Relaxed) {
            Some(pack_update_data(self.id, origin, &image, update)?)
        } else {
            None
        };

        // this is main lock for set and update operation
        let _lock = self.lock.lock();
        let mut w = self.image.write();

        if origin[0] + image.size[0] > w.size[0] || origin[1] + image.size[1] > w.size[1] {
            return Err("Image size exceeds bounds".to_string());
        }

        // update local image data
        let data_ptr = w.data.as_mut_ptr();
        unsafe {
            write_rectangle(
                image.data,
                image.stride,
                data_ptr,
                w.size[1],
                origin,
                &image.size,
                image.image_type,
            );
        }

        if let Some(to_send) = to_send {
            let new_rect = [origin[0], origin[1], image.size[0], image.size[1]];

            if let Some((rect, buffer)) = &mut w.buffer {
                if force && *rect == new_rect {
                    if self.event.is_set() {
                        match to_send {
                            SetDataUpdate::Single(data) => {
                                self.sender.send_single(data);
                                w.buffer = None;
                            }
                            SetDataUpdate::Multi(mut data) => {
                                if let Some((d, flag)) = data.pop_front() {
                                    self.sender.send_set(d, flag);
                                }
                                *buffer = data;
                            }
                        }
                    } else {
                        match to_send {
                            SetDataUpdate::Single(data) => {
                                buffer.clear();
                                buffer.push_back((data, true));
                            }
                            SetDataUpdate::Multi(data) => {
                                *buffer = data;
                            }
                        }
                    }
                } else {

                }
            } else {
            }
        }

        Ok(())
    }

    // Function is complex because it needs to handle different image types and also not contiguous
    // data. Also it tries to avoid copying data if possible.
    pub(crate) fn set_image_old(
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
        let data = if self.connected.load(Ordering::Relaxed) {
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
            let data_size = image.size[0] * image.size[1] * image.image_type.bytes_per_pixel();
            let header = ServerHeader::Image(self.id, update, image_header, data_size as u32);
            let buff = header
                .serialize_to_slice(&mut head_buff)
                .map_err(|_| "Failed to serialize image header")?;

            let offset = buff.len();

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

        // send the image to the server
        if let Some(data) = data {
            self.event.wait_clear();
            if !self.connected.load(Ordering::Relaxed) {
                return Ok(());
            }

            self.sender.send_single(SenderData::from_vec(data));
        }

        Ok(())
    }
}

impl Acknowledge for ValueImage {
    fn acknowledge(&self) {
        self.event.set();
    }

    fn reset(&self) {
        self.acknowledge();
    }
}

impl SyncTrait for ValueImage {
    fn sync(&self) -> Result<(), ()> {
        let w = self.image.read();
        if w.size[0] == 0 || w.size[1] == 0 {
            self.event.set();
            return Ok(());
        }

        let mut head_buff = [0u8; 64];
        let image_header = ImageHeader {
            image_size: [w.size[0] as u32, w.size[1] as u32],
            rect: None,
            image_type: ImageType::ColorAlpha,
        };
        let header = ServerHeader::Image(self.id, false, image_header, w.data.len() as u32);
        let buff = header.serialize_to_slice(&mut head_buff)?;

        let mut data = Vec::with_capacity(buff.len() + w.data.len());
        unsafe { data.set_len(buff.len() + w.data.len()) };
        data[..buff.len()].copy_from_slice(buff);
        data[buff.len()..].copy_from_slice(&w.data);
        drop(w);

        self.event.clear();
        self.sender.send_single(SenderData::from_vec(data));
        Ok(())
    }
}

fn pack_set_data(id: u64, image: &ImageData, update: bool) -> Result<SetData, String> {
    let size = [image.size[1] as u32, image.size[0] as u32]; // reverse for egui
    let bytes_size = image.size[0] * image.size[1] * image.image_type.bytes_per_pixel();
    let data = unsafe { std::slice::from_raw_parts(image.data, bytes_size) };

    if bytes_size <= MSG_SIZE_THRESHOLD {
        let header = ImageSetHeader::All(size, update);
        let mut message = header
            .serialize(id, image.image_type, bytes_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;

        message.reserve_exact(bytes_size);
        message.extend_from_slice(data);
        return Ok(SetData::Single(message, true));
    } else {
        let mut messages = Vec::new();
        let pixel_size = image.image_type.bytes_per_pixel();
        let pixel_count = image.size[0] * image.size[1];
        let chunk_pixels = MSG_SIZE_THRESHOLD / pixel_size;
        let chunk_size = chunk_pixels * pixel_size;
        let mut processed_pixels = 0;
        let mut processed = 0;

        let first_pixels = chunk_pixels.min(pixel_count);
        let first_size = first_pixels * pixel_size;
        let header = ImageSetHeader::Start(size, first_pixels as u32);
        let mut message = header
            .serialize(id, image.image_type, first_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;
        message.reserve_exact(first_size);
        message.extend_from_slice(&data[..first_size]);
        messages.push((message, true));
        processed_pixels += first_pixels;
        processed += first_size;

        while processed_pixels < pixel_count {
            let remaining_pixels = pixel_count - processed_pixels;
            if remaining_pixels <= chunk_pixels {
                let remaining_size = remaining_pixels * pixel_size;
                let header = ImageSetHeader::End(remaining_pixels as u32, update);
                let mut message = header
                    .serialize(id, image.image_type, remaining_size as u32)
                    .map_err(|_| format!("Failed to serialize header for image {}", id))?;
                message.extend_from_slice(&data[processed..]);
                messages.push((message, false));
                break;
            }

            let header = ImageSetHeader::Batch(chunk_pixels as u32);
            let mut message = header
                .serialize(id, image.image_type, chunk_size as u32)
                .map_err(|_| format!("Failed to serialize header for image {}", id))?;
            message.reserve_exact(chunk_size);
            message.extend_from_slice(&data[processed..processed + chunk_size]);
            messages.push((message, true));
            processed_pixels += chunk_pixels;
            processed += chunk_size;
        }

        return Ok(SetData::Multi(messages));
    }
}

fn pack_update_data(
    id: u64,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
) -> Result<SetDataUpdate, String> {
    let bytes_line_size = image.size[1] * image.image_type.bytes_per_pixel();
    let bytes_size = image.size[0] * bytes_line_size;

    let append_lines = |message: &mut FastVec<32>, start_line: usize, lines: usize| {
        if image.contiguous {
            let offset = start_line * bytes_line_size;
            let size = lines * bytes_line_size;
            let data = unsafe { std::slice::from_raw_parts(image.data.add(offset), size) };
            message.extend_from_slice(data);
        } else {
            for line in start_line..start_line + lines {
                let data = unsafe {
                    std::slice::from_raw_parts(image.data.add(line * image.stride), bytes_line_size)
                };
                message.extend_from_slice(data);
            }
        }
    };

    if bytes_size <= MSG_SIZE_THRESHOLD {
        // swap for egui
        let rect = [
            origin[1] as u32,
            origin[0] as u32,
            image.size[1] as u32,
            image.size[0] as u32,
        ];
        let header = ServerHeader::Image(
            id,
            ImageHeader::Update(rect, image.image_type, update),
            bytes_size as u32,
        );
        let mut message: FastVec<32> = crate::serialization::serialize_heap(&header)
            .map_err(|_| format!("Failed to serialize update header for image {}", id))?;

        message.reserve_exact(bytes_size);
        append_lines(&mut message, 0, image.size[0]);
        return Ok(SetDataUpdate::Single(message, true));
    } else {
        let mut messages = VecDeque::new();
        let chunk_lines = (MSG_SIZE_THRESHOLD / bytes_line_size).max(1);
        let mut processed_lines = 0;

        while processed_lines < image.size[0] {
            let remaining_lines = image.size[0] - processed_lines;
            let lines = remaining_lines.min(chunk_lines);
            let data_size = lines * bytes_line_size;
            let is_last = lines == remaining_lines;
            let rect = [
                (origin[0] + processed_lines) as u32,
                origin[1] as u32,
                lines as u32,
                image.size[1] as u32,
            ];
            let header = ServerHeader::Image(
                id,
                ImageHeader::Update(rect, image.image_type, is_last && update),
                data_size as u32,
            );
            let mut message: FastVec<32> = crate::serialization::serialize_heap(&header)
                .map_err(|_| format!("Failed to serialize update header for image {}", id))?;
            message.reserve_exact(data_size);
            append_lines(&mut message, processed_lines, lines);
            messages.push_front((message, !is_last));
            processed_lines += lines;
        }

        return Ok(SetDataUpdate::Multi(messages));
    }
}

unsafe fn write_all_new(
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

unsafe fn write_all_new_stride(
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
