use parking_lot::{Mutex, RwLock};
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;

use egui::{ColorImage, ImageData, TextureHandle};

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::image_transport::ImageType;

const TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Nearest,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

pub(crate) enum ImageSetMessage {
    All([u32; 2]),
    Start([u32; 2], u32),
    Batch(u32),
    End(u32),
}

pub(crate) enum ImageMessage {
    Set(ImageSetMessage, ImageType),
    Update([u32; 4], ImageType),
}

pub struct Image {
    name: Arc<String>,
    id: u64,
    inner: Arc<(RwLock<Option<(TextureHandle, [usize; 2])>>, MessageSender)>,
    buffer: Arc<Mutex<Option<(ColorImage, usize)>>>,
}

impl Image {
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            inner: Arc::new((RwLock::new(None), sender)),
            buffer: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get(&self) -> Option<(egui::TextureId, [usize; 2])> {
        self.inner
            .0
            .read()
            .as_ref()
            .map(|(texture_handle, size)| (texture_handle.id(), *size))
    }

    pub fn get_id(&self) -> Option<egui::TextureId> {
        self.inner
            .0
            .read()
            .as_ref()
            .map(|(texture_handle, _)| texture_handle.id())
    }

    pub fn get_size(&self) -> Option<[usize; 2]> {
        self.inner.0.read().as_ref().map(|(_, size)| *size)
    }

    pub fn initialize(&self, ctx: &egui::Context, image: ColorImage) {
        let image_data = ImageData::Color(Arc::new(image));
        let name = format!("image_{}", self.id);
        let texture_handle = ctx.load_texture(name, image_data, TEXTURE_OPTIONS);

        let mut w = self.inner.0.write();
        let size = texture_handle.size();
        match *w {
            None => {
                *w = Some((texture_handle, size));
            }
            _ => {}
        }
    }

    pub(crate) fn set_image(
        &self,
        message: ImageSetMessage,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        match message {
            ImageSetMessage::All(size) => {
                self.inner.1.send(ChannelMessage::Ack(self.id));
                let image_size = [size[0] as usize, size[1] as usize];
                if image_type.bytes_per_pixel() * image_size[0] * image_size[1] != data.len() {
                    return Err(format!(
                        "Data length does not match expected size: {}",
                        data.len()
                    ));
                }

                let c_image = self.create_c_image(image_size, image_type, data)?;
                if let Some((ref mut texture_handle, ref mut save_size)) = *self.inner.0.write() {
                    texture_handle.set(c_image, TEXTURE_OPTIONS);
                    *save_size = image_size;
                }
            }
            ImageSetMessage::Start(size, pixels) => {
                let pixels = pixels as usize;
                let size = [size[0] as usize, size[1] as usize];
                let mut c_image = ColorImage::filled(size, egui::Color32::WHITE);
                self.update_c_image(&mut c_image, 0, pixels, data, image_type)?;
                *self.buffer.lock() = Some((c_image, pixels))
            }
            ImageSetMessage::Batch(pixels) => {
                let pixels = pixels as usize;
                if let Some((ref mut c_image, ref mut actual_pixel)) = *self.buffer.lock() {
                    let actual = *actual_pixel as usize;

                    if actual + pixels >= c_image.pixels.len() {
                        return Err(format!("Pixels exceed image size in {}", self.name));
                    }

                    self.update_c_image(c_image, actual, pixels, data, image_type)?;
                    *actual_pixel += pixels;
                } else {
                    return Err(format!("No image buffer found for image: {}", self.name));
                }
            }
            ImageSetMessage::End(pixels) => {
                self.inner.1.send(ChannelMessage::Ack(self.id));
                let pixels = pixels as usize;
                if let Some((c_image, actual_pixel)) = self.buffer.lock().take() {
                    if actual_pixel + pixels != c_image.pixels.len() {
                        return Err(format!(
                            "Pixels do not match expected size in {}: {} vs {}",
                            self.name,
                            actual_pixel + pixels,
                            c_image.pixels.len()
                        ));
                    }

                    self.update_c_image(
                        &mut c_image.clone(),
                        actual_pixel,
                        pixels,
                        data,
                        image_type,
                    )?;

                    if let Some((ref mut texture_handle, ref mut save_size)) = *self.inner.0.write()
                    {
                        let size = [c_image.width(), c_image.height()];
                        texture_handle.set(c_image, TEXTURE_OPTIONS);
                        *save_size = size;
                    }
                } else {
                    return Err(format!("No image buffer found for image: {}", self.name));
                }
            }
        }

        Ok(())
    }

    pub(crate) fn update_image(
        &self,
        rect: [u32; 4],
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        // TODO: not sure if this is the best place to send ack
        self.inner.1.send(ChannelMessage::Ack(self.id));

        let image_size = [rect[2] as usize, rect[3] as usize];
        let origin = [rect[0] as usize, rect[1] as usize];

        if image_type.bytes_per_pixel() * image_size[0] * image_size[1] != data.len() {
            return Err(format!(
                "Data length does not match expected size: {}",
                self.name
            ));
        }

        let c_image = self.create_c_image(image_size, image_type, data)?;

        let mut w = self.inner.0.write();
        if let Some((ref mut texture_handle, ref mut save_size)) = *w {
            if *save_size == image_size && origin == [0, 0] {
                texture_handle.set(c_image, TEXTURE_OPTIONS);
            } else {
                if origin[0] + image_size[0] > save_size[0]
                    || origin[1] + image_size[1] > save_size[1]
                {
                    return Err(format!(
                        "Image is larger than the texture for image: {}",
                        self.name
                    ));
                }
                texture_handle.set_partial(origin, c_image, TEXTURE_OPTIONS);
            }
        }

        Ok(())
    }

    fn update_c_image(
        &self,
        image: &mut ColorImage,
        actual_pixel: usize,
        pixels: usize,
        data: &[u8],
        image_type: ImageType,
    ) -> Result<(), String> {
        if actual_pixel + pixels > image.pixels.len() {
            return Err(format!("Pixels exceed image size in {}", self.name));
        }

        if image_type.bytes_per_pixel() * pixels != data.len() {
            return Err(format!(
                "Data length does not match expected size in {}",
                self.name
            ));
        }

        let data_ptr = data.as_ptr();
        let image_ptr = unsafe { image.pixels.as_mut_ptr().add(actual_pixel) as *mut u8 };

        unsafe {
            fill_c_image(image_type, data_ptr, image_ptr, pixels);
        }

        Ok(())
    }

    fn create_c_image(
        &self,
        image_size: [usize; 2],
        image_type: ImageType,
        data: &[u8],
    ) -> Result<ColorImage, String> {
        if image_type.bytes_per_pixel() * image_size[0] * image_size[1] != data.len() {
            return Err(format!(
                "Data length does not match expected size in {}",
                self.name
            ));
        }

        let mut c_image = ColorImage::filled(image_size, egui::Color32::TRANSPARENT);
        let pixel_count = image_size[0] * image_size[1];

        let data_ptr = data.as_ptr();
        let image_ptr = c_image.pixels.as_mut_ptr() as *mut u8;

        unsafe { fill_c_image(image_type, data_ptr, image_ptr, pixel_count) }

        Ok(c_image)
    }
}

impl Clone for Image {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            inner: self.inner.clone(),
            buffer: self.buffer.clone(),
        }
    }
}

unsafe fn fill_c_image(
    image_type: ImageType,
    data_ptr: *const u8,
    image_ptr: *mut u8,
    pixel_count: usize,
) {
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
}
