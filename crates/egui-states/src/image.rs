use parking_lot::RwLock;
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;

use egui::{ColorImage, ImageData, TextureHandle};

use egui_states_core::image::{ImageInfo, ImageType};

use crate::UpdateValue;

const TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Nearest,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

pub struct ValueImage {
    id: u32,
    texture_handle: RwLock<Option<(TextureHandle, [usize; 2])>>,
}

impl ValueImage {
    pub fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            texture_handle: RwLock::new(None),
        })
    }

    pub fn get_id(&self) -> egui::TextureId {
        self.texture_handle
            .read()
            .as_ref()
            .expect("image is not initialized")
            .0
            .id()
    }

    pub fn get_size(&self) -> [usize; 2] {
        self.texture_handle
            .read()
            .as_ref()
            .expect("image is not initialized")
            .1
    }

    pub fn initialize(&self, ctx: &egui::Context, image: ColorImage) {
        let image_data = ImageData::Color(Arc::new(image));
        let name = format!("image_{}", self.id);
        let texture_handle = ctx.load_texture(name, image_data, TEXTURE_OPTIONS);

        let mut w = self.texture_handle.write();
        let size = texture_handle.size();
        match *w {
            None => {
                *w = Some((texture_handle, size));
            }
            _ => {}
        }
    }
}

impl UpdateValue for ValueImage {
    fn update_value(&self, data: &[u8]) -> Result<bool, String> {
        let (info, dat) = ImageInfo::deserialize(data).map_err(|e| {
            format!(
                "Failed to deserialize image message: {} for image of id {}",
                e, self.id
            )
        })?;

        let ImageInfo {
            image_size,
            rect,
            image_type,
            update,
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

        let mut c_image = egui::ColorImage::filled(size, egui::Color32::WHITE);
        let pixel_count = size[0] * size[1];

        let data_ptr = dat.as_ptr();
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

        let mut w = self.texture_handle.write();
        if let Some((ref mut texture_handle, ref mut save_size)) = *w {
            match rect {
                Some(rec) => {
                    if save_size[0] != image_size[1] || save_size[1] != image_size[0] {
                        return Err(
                            "Rectangle is set but the image size is different from texture"
                                .to_string(),
                        );
                    }
                    texture_handle.set_partial([rec[1], rec[0]], c_image, TEXTURE_OPTIONS);
                }
                None => {
                    texture_handle.set(c_image, TEXTURE_OPTIONS);
                    *save_size = size;
                }
            }
        }

        Ok(update)
    }
}
