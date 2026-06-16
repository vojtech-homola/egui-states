use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::event::Event;
use crate::image_transport::{ImageHeader, ImageSetHeader, ImageType};
use crate::serialization::ServerHeader;
use crate::serialization::{FastVec, MSG_SIZE_THRESHOLD};
use crate::server::sender::MessageSender;
use crate::server::server::{Acknowledge, SyncTrait};

enum Buffer {
    Set(Vec<(FastVec<32>, bool)>),
    Update([usize; 4], VecDeque<(FastVec<32>, bool)>),
}

struct ImageDataInner {
    data: Vec<u8>,
    size: [usize; 2],
    buffer: Buffer,
}

pub(crate) struct ImageData {
    pub size: [usize; 2],
    pub stride: usize,
    pub contiguous: bool,
    pub image_type: ImageType,
    pub data: *const u8,
}

pub(crate) struct Image {
    pub(crate) name: String,
    id: u64,
    image: RwLock<ImageDataInner>,
    lock: Mutex<()>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    event: Event,
}

impl Image {
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
                buffer: Buffer::Set(Vec::new()),
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
        let lock = self.lock.lock();
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

        let Some(to_send) = to_send else {
            return Ok(());
        };

        if let Buffer::Set(ref mut dat) = w.buffer {
            if dat.is_empty() {
                if self.event.is_set() {
                    self.event.clear();
                    for (message, send_now) in to_send {
                        self.sender.send_set(message, send_now);
                    }
                    return Ok(());
                } else {
                    dat.extend(to_send);
                    return Ok(());
                }
            }
        } else {
            //rewrite any update buffer, set has always priority over update
            if self.event.is_set() {
                self.event.clear();
                for (message, send_now) in to_send {
                    self.sender.send_set(message, send_now);
                }
                return Ok(());
            } else {
                w.buffer = Buffer::Set(to_send);
                return Ok(());
            }
        }

        // drop lock to allow acknowledge to process the buffer and send data while we prepare the next one
        drop(w);

        self.event.wait_clear();
        if !self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        for (message, send_now) in to_send {
            self.sender.send_set(message, send_now);
        }

        drop(lock);

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

        let Some(mut to_send) = to_send else {
            return Ok(());
        };

        let new_rect = [origin[0], origin[1], image.size[0], image.size[1]];

        match w.buffer {
            Buffer::Update(ref mut rect, ref mut dat) => {
                if dat.is_empty() {
                    *rect = new_rect;
                    if self.event.is_set() {
                        self.event.clear();
                        if let Some((message, send_now)) = to_send.pop_front() {
                            self.sender.send_set(message, send_now);
                        }

                        if !to_send.is_empty() {
                            dat.extend(to_send);
                        }

                        return Ok(());
                    } else {
                        dat.extend(to_send);
                        return Ok(());
                    }
                }

                if force && new_rect == *rect {
                    dat.clear();
                    dat.extend(to_send);
                    return Ok(());
                }
            }
            Buffer::Set(ref dat) => {
                if dat.is_empty() {
                    if self.event.is_set() {
                        self.event.clear();
                        if let Some((message, send_now)) = to_send.pop_front() {
                            self.sender.send_set(message, send_now);
                        }
                        w.buffer = Buffer::Update(new_rect, to_send);

                        return Ok(());
                    } else {
                        w.buffer = Buffer::Update(new_rect, to_send);
                        return Ok(());
                    }
                }
            }
        }

        drop(w);

        self.event.wait_clear();
        if !self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        let mut w = self.image.write();
        if let Some((message, send_now)) = to_send.pop_front() {
            self.sender.send_set(message, send_now);
        }
        w.buffer = Buffer::Update(new_rect, to_send);

        Ok(())
    }
}

impl Acknowledge for Image {
    fn acknowledge(&self) {
        let mut w = self.image.write();

        match w.buffer {
            Buffer::Set(ref mut dat) => {
                if dat.is_empty() {
                    self.event.set();
                    return;
                }

                for (message, send_now) in dat.drain(..) {
                    self.sender.send_set(message, send_now);
                }
            }
            Buffer::Update(_, ref mut dat) => match dat.pop_front() {
                Some((message, send_now)) => {
                    self.sender.send_set(message, send_now);
                }
                None => {
                    self.event.set();
                    return;
                }
            },
        }
    }

    fn reset(&self) {
        self.event.set();
        let mut w = self.image.write();
        w.buffer = Buffer::Set(Vec::new());
    }
}

impl SyncTrait for Image {
    fn sync(&self) -> Result<(), ()> {
        let mut w = self.image.write();
        if w.size[0] == 0 || w.size[1] == 0 {
            self.event.set();
            return Ok(());
        }

        let image_data = ImageData {
            size: w.size,
            stride: 0,
            contiguous: true,
            image_type: ImageType::ColorAlpha,
            data: w.data.as_ptr(),
        };

        let data = pack_set_data(self.id, &image_data, false).map_err(|_| ())?;

        w.buffer = Buffer::Set(Vec::new());
        self.event.clear();
        for (message, send_now) in data {
            self.sender.send_set(message, send_now);
        }
        Ok(())
    }
}

fn pack_set_data(
    id: u64,
    image: &ImageData,
    update: bool,
) -> Result<Vec<(FastVec<32>, bool)>, String> {
    let size = [image.size[1] as u32, image.size[0] as u32]; // reverse for egui
    let bytes_line_size = image.size[1] * image.image_type.bytes_per_pixel();
    let bytes_size = image.size[0] * bytes_line_size;

    let append_data = |message: &mut FastVec<32>, start: usize, size: usize| {
        if image.contiguous {
            let data = unsafe { std::slice::from_raw_parts(image.data.add(start), size) };
            message.extend_from_slice(data);
        } else {
            let mut processed = 0;
            while processed < size {
                let offset = start + processed;
                let line = offset / bytes_line_size;
                let line_offset = offset % bytes_line_size;
                let copy_size = (bytes_line_size - line_offset).min(size - processed);
                let data = unsafe {
                    std::slice::from_raw_parts(
                        image.data.add(line * image.stride + line_offset),
                        copy_size,
                    )
                };
                message.extend_from_slice(data);
                processed += copy_size;
            }
        }
    };

    if bytes_size <= MSG_SIZE_THRESHOLD {
        let header = ImageSetHeader::All(size, update);
        let mut message = header
            .serialize(id, image.image_type, bytes_size as u32)
            .map_err(|_| format!("Failed to serialize header for image {}", id))?;

        message.reserve_exact(bytes_size);
        append_data(&mut message, 0, bytes_size);
        return Ok(vec![(message, true)]);
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
        append_data(&mut message, 0, first_size);
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
                append_data(&mut message, processed, remaining_size);
                messages.push((message, false));
                break;
            }

            let header = ImageSetHeader::Batch(chunk_pixels as u32);
            let mut message = header
                .serialize(id, image.image_type, chunk_size as u32)
                .map_err(|_| format!("Failed to serialize header for image {}", id))?;
            message.reserve_exact(chunk_size);
            append_data(&mut message, processed, chunk_size);
            messages.push((message, true));
            processed_pixels += chunk_pixels;
            processed += chunk_size;
        }

        return Ok(messages);
    }
}

fn pack_update_data(
    id: u64,
    origin: &[usize; 2],
    image: &ImageData,
    update: bool,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
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
        let mut result = VecDeque::with_capacity(1);
        result.push_back((message, update));
        return Ok(result);
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
                origin[1] as u32,
                (origin[0] + processed_lines) as u32,
                image.size[1] as u32,
                lines as u32,
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
            messages.push_back((message, !is_last));
            processed_lines += lines;
        }

        return Ok(messages);
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
