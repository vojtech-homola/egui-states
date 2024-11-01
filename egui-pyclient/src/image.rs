use std::ptr::copy_nonoverlapping;
use std::sync::{Arc, RwLock};

use egui::{Color32, ColorImage, ImageData, TextureHandle};

use egui_pytransport::image::{ImageMessage, ImageType};

pub(crate) trait ImageUpdate: Send + Sync {
    fn update_image(&self, message: ImageMessage) -> Result<(), String>;
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
    fn update_image(&self, message: ImageMessage) -> Result<(), String> {
        // let message = match message {
        //     ImageMessage::Data(data) => data,
        //     ImageMessage::Histogram(histogram) => {
        //         *self.histogram.write().unwrap() = (histogram, true);
        //         return Ok(());
        //     }
        // };

        let ImageMessage {
            image_size,
            rect,
            data,
            image_type,
        } = message;

        let actual_size = self.texture_handle.read().unwrap().1;
        if actual_size != image_size && rect.is_some() {
            return Err(
                "Rectangle is set but the image size is different from texture".to_string(),
            );
        }

        let size = match rect {
            Some(r) => [r[3], r[2]],
            None => [image_size[1], image_size[0]],
        };

        // TODO: cache the color image
        let mut c_image = egui::ColorImage::new(size, egui::Color32::WHITE);
        let pixel_count = size[0] * size[1];

        let data_ptr = data.as_ptr();
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
        if let Some(ref mut texture_handle) = w.0 {
            match rect {
                Some(rec) => texture_handle.set_partial([rec[1], rec[0]], c_image, TEXTURE_OPTIONS),
                None => texture_handle.set(c_image, TEXTURE_OPTIONS),
            }

            w.1 = size;
        }

        Ok(())
    }
}
