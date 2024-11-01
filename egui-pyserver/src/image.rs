use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use egui_pytransport::image::{ImageMessage, ImageType};
use egui_pytransport::transport::WriteMessage;

use crate::SyncTrait;

struct ImageData {
    data: Vec<u8>,
    size: [usize; 2],
}

pub struct ValueImage {
    id: u32,
    image: RwLock<ImageData>,
    channel: Sender<WriteMessage>,
    connected: Arc<AtomicBool>,
}

impl ValueImage {
    pub(crate) fn new(
        id: u32,
        channel: Sender<WriteMessage>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            image: RwLock::new(ImageData {
                data: Vec::with_capacity(0),
                size: [0, 0],
            }),
            channel,
            connected,
        })
    }

    pub(crate) fn set_image_py(
        &self,
        data: Vec<u8>,
        shape: Vec<usize>,
        rectangle: Option<[usize; 4]>,
        update: bool,
    ) -> PyResult<()> {
        let image_type = match shape.len() {
            2 => ImageType::Gray,
            3 => match shape[2] {
                2 => ImageType::GrayAlpha,
                3 => ImageType::Color,
                4 => ImageType::ColorAlpha,
                _ => return Err(PyValueError::new_err("Invalid image dimensions")),
            },
            _ => return Err(PyValueError::new_err("Invalid image dimensions")),
        };
        let size = [shape[0], shape[1]];

        let mut w = self.image.write().unwrap();
        match rectangle {
            Some(rect) => {
                if size != [rect[2], rect[3]] {
                    return Err(PyValueError::new_err(format!(
                        "image size {:?} does not match rectangle size {:?}",
                        size,
                        [rect[2], rect[3]]
                    )));
                }

                if rect[2] == 0 || rect[3] == 0 {
                    return Err(PyValueError::new_err("Rctangle size cannot be zero"));
                }

                let original_size = w.size;
                if rect[0] + rect[2] > original_size[0] || rect[1] + rect[3] > original_size[1] {
                    return Err(PyValueError::new_err(format!(
                        "rectangle {:?} does not fit  int the image with size {:?}",
                        rect, original_size
                    )));
                }

                match image_type {
                    ImageType::ColorAlpha => {
                        for i in 0..rect[2] {
                            for j in 0..rect[3] {
                                let index = (rect[0] + i) * original_size[1] + rect[1] + j;
                                let d_index = i * j * 4;
                                w.data[index * 4] = data[d_index];
                                w.data[index * 4 + 1] = data[d_index + 1];
                                w.data[index * 4 + 2] = data[d_index + 2];
                                w.data[index * 4 + 3] = data[d_index + 3];
                            }
                        }
                    }
                    ImageType::Color => {
                        for i in 0..rect[2] {
                            for j in 0..rect[3] {
                                let index = (rect[0] + i) * original_size[1] + rect[1] + j;
                                let d_index = i * j * 3;
                                w.data[index * 4] = data[d_index];
                                w.data[index * 4 + 1] = data[d_index + 1];
                                w.data[index * 4 + 2] = data[d_index + 2];
                                w.data[index * 4 + 3] = 255;
                            }
                        }
                    }
                    ImageType::Gray => {
                        for i in 0..rect[2] {
                            for j in 0..rect[3] {
                                let index = (rect[0] + i) * original_size[1] + rect[1] + j;
                                let p = data[i * j];
                                w.data[index * 4] = p;
                                w.data[index * 4 + 1] = p;
                                w.data[index * 4 + 2] = p;
                                w.data[index * 4 + 3] = 255;
                            }
                        }
                    }
                    ImageType::GrayAlpha => {
                        for i in 0..rect[2] {
                            for j in 0..rect[3] {
                                let index = (rect[0] + i) * original_size[1] + rect[1] + j;
                                let d_index = i * j * 2;
                                let p = data[d_index];
                                w.data[index * 4] = p;
                                w.data[index * 4 + 1] = p;
                                w.data[index * 4 + 2] = p;
                                w.data[index * 4 + 3] = data[d_index + 1];
                            }
                        }
                    }
                }
            }
            None => {
                w.size = size;
                match image_type {
                    ImageType::ColorAlpha => w.data = data.clone(),
                    ImageType::Color => {
                        let mut new_data = vec![0u8; size[0] * size[1] * 4];
                        for i in 0..size[0] * size[1] {
                            new_data[i * 4] = data[i * 3];
                            new_data[i * 4 + 1] = data[i * 3 + 1];
                            new_data[i * 4 + 2] = data[i * 3 + 2];
                            new_data[i * 4 + 3] = 255;
                        }
                        w.data = new_data;
                    }
                    ImageType::Gray => {
                        let mut new_data = vec![0u8; size[0] * size[1] * 4];
                        for i in 0..size[0] * size[1] {
                            let p = data[i];
                            new_data[i * 4] = p;
                            new_data[i * 4 + 1] = p;
                            new_data[i * 4 + 2] = p;
                            new_data[i * 4 + 3] = 255;
                        }
                        w.data = new_data;
                    }

                    ImageType::GrayAlpha => {
                        let mut new_data = vec![0u8; size[0] * size[1] * 4];
                        for i in 0..size[0] * size[1] {
                            let p = data[i * 2];
                            new_data[i * 4] = p;
                            new_data[i * 4 + 1] = p;
                            new_data[i * 4 + 2] = p;
                            new_data[i * 4 + 3] = data[i * 2 + 1];
                        }
                        w.data = new_data;
                    }
                }
            }
        }
        let new_size = w.size;

        if self.connected.load(Ordering::Relaxed) {
            let image_message = ImageMessage {
                image_size: new_size,
                rect: rectangle,
                data,
                image_type,
            };

            let message = WriteMessage::Image(self.id, update, image_message);
            self.channel.send(message).unwrap();
        }

        Ok(())
    }
}

impl SyncTrait for ValueImage {
    fn sync(&self) {
        let w = self.image.read().unwrap();
        if w.size[0] == 0 || w.size[1] == 0 {
            return;
        }

        let image_message = ImageMessage {
            image_size: w.size,
            rect: None,
            data: w.data.clone(),
            image_type: ImageType::ColorAlpha,
        };
        drop(w);

        let message = WriteMessage::Image(self.id, false, image_message);
        self.channel.send(message).unwrap();
    }
}
