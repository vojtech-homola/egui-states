use std::io::Read;
use std::net::TcpStream;
use std::ptr::copy_nonoverlapping;
use std::sync::{Arc, RwLock};

use egui::{Color32, ColorImage, ImageData, TextureHandle};

use egui_pysync_common::transport::{self, ImageDataMessage, ImageMessage, ImageType, ParseError};

pub(crate) fn read_image_message(
    head: &mut [u8],
    stream: &mut TcpStream,
) -> Result<ImageMessage, ParseError> {
    let subtype = head[6];
    match subtype {
        transport::IMAGE_DATA => read_image_data(head, stream),
        transport::IMAGE_HISTOGRAM => {
            let histogram_size = u16::from_le_bytes(head[7..9].try_into().unwrap()) as usize;

            let data = if histogram_size > 0 {
                let mut data = vec![0f32; histogram_size];
                let data_u8 = data.as_mut_ptr() as *mut u8;
                let data_buff = unsafe {
                    std::slice::from_raw_parts_mut(data_u8, histogram_size * size_of::<f32>())
                };

                stream
                    .read_exact(data_buff)
                    .map_err(|e| ParseError::Connection(e))?;
                Some(data)
            } else {
                None
            };

            Ok(ImageMessage::Histogram(data))
        }
        _ => Err(ParseError::Parse(
            "Unknown image message subtype".to_string(),
        )),
    }
}

fn read_image_data(head: &mut [u8], stream: &mut TcpStream) -> Result<ImageMessage, ParseError> {
    let image_type = match head[7] {
        transport::IMAGE_COLOR => ImageType::Color,
        transport::IMAGE_GRAY => ImageType::Gray,
        transport::IMAGE_COLOR_ALPHA => ImageType::ColorAlpha,
        transport::IMAGE_GRAY_ALPHA => ImageType::GrayAlpha,
        _ => return Err(ParseError::Parse("Unknown image type".to_string())),
    };

    let y = u16::from_le_bytes(head[8..10].try_into().unwrap()) as usize;
    let x = u16::from_le_bytes(head[10..12].try_into().unwrap()) as usize;
    let is_rectangle = head[12] != 0;

    let (rectangle, data_size) = if is_rectangle {
        let rectangle = [
            u16::from_le_bytes(head[13..15].try_into().unwrap()) as usize,
            u16::from_le_bytes(head[15..17].try_into().unwrap()) as usize,
            u16::from_le_bytes(head[17..19].try_into().unwrap()) as usize,
            u16::from_le_bytes(head[19..21].try_into().unwrap()) as usize,
        ];
        let data_size = rectangle[2] * rectangle[3];
        (Some(rectangle), data_size)
    } else {
        (None, y * x)
    };
    let size = u64::from_le_bytes(head[21..29].try_into().unwrap()) as usize;
    let histogram_size = u16::from_le_bytes(head[29..31].try_into().unwrap()) as usize;

    let is_right = match image_type {
        ImageType::Color => size == data_size * 3,
        ImageType::ColorAlpha => size == data_size * 4,
        ImageType::Gray => size == data_size,
        ImageType::GrayAlpha => size == data_size * 2,
    };

    if !is_right {
        return Err(ParseError::Parse(format!(
            "Wrong size of the image data: {}",
            size
        )));
    }

    let mut data = vec![0u8; size as usize];
    stream
        .read_exact(&mut data)
        .map_err(|e| ParseError::Connection(e))?;

    let histogram = if histogram_size > 0 {
        let mut hist = vec![0f32; histogram_size];
        let hist_u8 = hist.as_mut_ptr() as *mut u8;
        let hist_buff =
            unsafe { std::slice::from_raw_parts_mut(hist_u8, histogram_size * size_of::<f32>()) };

        stream
            .read_exact(hist_buff)
            .map_err(|e| ParseError::Connection(e))?;
        Some(hist)
    } else {
        None
    };

    let image_data = ImageDataMessage {
        image_size: [y, x],
        rect: rectangle,
        data,
        image_type,
        histogram,
    };

    Ok(ImageMessage::Data(image_data))
}

pub(crate) trait ImageUpdate: Send + Sync {
    fn update_image(&self, message: ImageMessage) -> Result<(), String>;
}

const TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Nearest,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

pub struct ImageValue {
    id: u32,
    texture_handle: RwLock<(Option<TextureHandle>, [usize; 2])>,
    histogram: RwLock<(Option<Vec<f32>>, bool)>,
}

impl ImageValue {
    pub fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            id,
            texture_handle: RwLock::new((None, [0, 0])),
            histogram: RwLock::new((None, true)),
        })
    }

    pub fn get_id(&self) -> egui::TextureId {
        self.texture_handle.read().unwrap().0.as_ref().unwrap().id()
    }

    pub fn get_size(&self) -> [usize; 2] {
        self.texture_handle.read().unwrap().1
    }

    pub fn get_histogram(&self) -> Option<Vec<f32>> {
        self.histogram.read().unwrap().0.clone()
    }

    pub fn process_histogram(&self, op: impl Fn(Option<&Vec<f32>>, bool)) {
        let mut w = self.histogram.write().unwrap();
        op(w.0.as_ref(), w.1);
        w.1 = false; // TODO: use separate flag?
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

impl ImageUpdate for ImageValue {
    fn update_image(&self, message: ImageMessage) -> Result<(), String> {
        let message = match message {
            ImageMessage::Data(data) => data,
            ImageMessage::Histogram(histogram) => {
                *self.histogram.write().unwrap() = (histogram, true);
                return Ok(());
            }
        };

        let ImageDataMessage {
            image_size,
            rect,
            data,
            image_type,
            histogram,
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

        if let Some(hist) = histogram {
            *self.histogram.write().unwrap() = (Some(hist), true);
        }

        Ok(())
    }
}
