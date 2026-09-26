//! Client image state backed by an egui texture.

use parking_lot::{Mutex, RwLock};
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;

use egui::{ColorImage, ImageData, TextureHandle};

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::hashing::NoHashMap;
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
    Fill([u32; 2], [u8; 4]),
}

pub(crate) enum ImageMultiMessage {
    Remove(u32),
    Modify(u32, ImageMessage),
    Reset,
}

impl ImageMultiMessage {
    pub(crate) fn requires_ack(&self) -> bool {
        matches!(self, Self::Modify(_, message) if message.requires_ack())
    }
}

impl ImageMessage {
    pub(crate) fn requires_ack(&self) -> bool {
        matches!(
            self,
            Self::Set(ImageSetMessage::All(_) | ImageSetMessage::End(_), _)
                | Self::Update(..)
                | Self::Fill(..)
        )
    }
}

/// A server-controlled RGBA texture used by the egui client.
///
/// Call [`Self::initialize`] once with an egui context before expecting server
/// image updates to become visible. Updates received before initialization are
/// validated and acknowledged but cannot be applied to a texture.
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

    /// Returns the texture identifier and `[width, height]`, if initialized.
    pub fn get(&self) -> Option<(egui::TextureId, [usize; 2])> {
        self.inner
            .0
            .read()
            .as_ref()
            .map(|(texture_handle, size)| (texture_handle.id(), *size))
    }

    /// Returns the texture identifier, if initialized.
    pub fn get_id(&self) -> Option<egui::TextureId> {
        self.inner
            .0
            .read()
            .as_ref()
            .map(|(texture_handle, _)| texture_handle.id())
    }

    /// Returns the texture size as `[width, height]`, if initialized.
    pub fn get_size(&self) -> Option<[usize; 2]> {
        self.inner.0.read().as_ref().map(|(_, size)| *size)
    }

    /// Creates the egui texture with an initial image.
    ///
    /// Call this from the UI before connecting when the server may send an image
    /// immediately. Subsequent calls are ignored after initialization, even if
    /// they use a different context or image. The texture uses nearest-neighbor
    /// filtering and clamps sampling at its edges.
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
                if let Some((mut c_image, actual_pixel)) = self.buffer.lock().take() {
                    if actual_pixel + pixels != c_image.pixels.len() {
                        return Err(format!(
                            "Pixels do not match expected size in {}: {} vs {}",
                            self.name,
                            actual_pixel + pixels,
                            c_image.pixels.len()
                        ));
                    }

                    self.update_c_image(&mut c_image, actual_pixel, pixels, data, image_type)?;

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

    pub(crate) fn fill_image(
        &self,
        size: [u32; 2],
        rgba: [u8; 4],
        data: &[u8],
    ) -> Result<(), String> {
        if !data.is_empty() {
            self.inner.1.send_ack(self.id);
            return Err(format!(
                "Fill message for image {} contains unexpected pixel data",
                self.name
            ));
        }
        if size[0] == 0 || size[1] == 0 {
            self.inner.1.send_ack(self.id);
            return Err(format!(
                "Fill message for image {} contains zero dimensions",
                self.name
            ));
        }

        let image_size = [size[0] as usize, size[1] as usize];
        if image_size[0].checked_mul(image_size[1]).is_none() {
            self.inner.1.send_ack(self.id);
            return Err(format!(
                "Image dimensions overflow for image: {}",
                self.name
            ));
        }

        self.inner.1.send(ChannelMessage::Ack(self.id));

        let [red, green, blue, alpha] = rgba;
        let pixel = egui::Color32::from_rgba_premultiplied(red, green, blue, alpha);
        let c_image = ColorImage::filled(image_size, pixel);
        if let Some((ref mut texture_handle, ref mut save_size)) = *self.inner.0.write() {
            texture_handle.set(c_image, TEXTURE_OPTIONS);
            *save_size = image_size;
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

struct ImageMultiInner {
    context: Option<egui::Context>,
    images: NoHashMap<u32, (TextureHandle, [usize; 2])>,
}

/// A server-controlled sparse collection of RGBA textures indexed by `u32` keys.
///
/// Call [`Self::initialize`] once with an egui context before expecting server
/// images to become visible. Complete images received for absent keys create
/// their textures automatically. Images received before initialization are
/// validated and acknowledged but cannot be applied to a texture.
pub struct ImageMulti {
    name: Arc<String>,
    id: u64,
    inner: Arc<RwLock<ImageMultiInner>>,
    buffers: Arc<Mutex<NoHashMap<u32, (ColorImage, usize)>>>,
    sender: MessageSender,
}

impl ImageMulti {
    pub(crate) fn new(name: String, id: u64, sender: MessageSender) -> Self {
        Self {
            name: Arc::new(name),
            id,
            inner: Arc::new(RwLock::new(ImageMultiInner {
                context: None,
                images: NoHashMap::default(),
            })),
            buffers: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
        }
    }

    /// Stores the egui context used to create textures for newly populated keys.
    ///
    /// Call this from the UI before connecting when the server may send images
    /// immediately. Subsequent calls are ignored after initialization, even if
    /// they use a different context. Newly created textures use nearest-neighbor
    /// filtering and clamp sampling at their edges.
    pub fn initialize(&self, ctx: &egui::Context) {
        let mut inner = self.inner.write();
        if inner.context.is_none() {
            inner.context = Some(ctx.clone());
        }
    }

    /// Returns the texture identifier and `[width, height]` for `index`, if
    /// populated.
    pub fn get(&self, index: u32) -> Option<(egui::TextureId, [usize; 2])> {
        self.inner
            .read()
            .images
            .get(&index)
            .map(|(texture, size)| (texture.id(), *size))
    }

    /// Returns the texture identifier for `index`, if populated.
    pub fn get_id(&self, index: u32) -> Option<egui::TextureId> {
        self.get(index).map(|(id, _)| id)
    }

    /// Returns the `[width, height]` texture size for `index`, if populated.
    pub fn get_size(&self, index: u32) -> Option<[usize; 2]> {
        self.get(index).map(|(_, size)| size)
    }

    /// Returns the number of populated keys, not the highest index plus one.
    pub fn len(&self) -> usize {
        self.inner.read().images.len()
    }

    /// Returns whether the collection contains no images.
    pub fn is_empty(&self) -> bool {
        self.inner.read().images.is_empty()
    }

    /// Returns whether `index` currently has an image.
    pub fn contains(&self, index: u32) -> bool {
        self.inner.read().images.contains_key(&index)
    }

    /// Returns a snapshot of the populated indices in ascending order.
    pub fn indices(&self) -> Vec<u32> {
        let mut indices = self.inner.read().images.keys().copied().collect::<Vec<_>>();
        indices.sort_unstable();
        indices
    }

    pub(crate) fn set_image(
        &self,
        index: u32,
        message: ImageSetMessage,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        match message {
            ImageSetMessage::All(size) => {
                self.sender.send_ack(self.id);
                let image_size = [size[0] as usize, size[1] as usize];
                let image = create_color_image(&self.name, image_size, image_type, data)?;
                self.set_texture(index, image)
            }
            ImageSetMessage::Start(size, pixels) => {
                let pixels = pixels as usize;
                let size = [size[0] as usize, size[1] as usize];
                let mut image = ColorImage::filled(size, egui::Color32::WHITE);
                update_color_image(&self.name, &mut image, 0, pixels, data, image_type)?;
                self.buffers.lock().insert(index, (image, pixels));
                Ok(())
            }
            ImageSetMessage::Batch(pixels) => {
                let pixels = pixels as usize;
                let mut buffers = self.buffers.lock();
                let (image, actual_pixel) = buffers.get_mut(&index).ok_or_else(|| {
                    format!(
                        "No image buffer found for image collection: {} index {}",
                        self.name, index
                    )
                })?;
                if *actual_pixel + pixels >= image.pixels.len() {
                    return Err(format!(
                        "Pixels exceed image size in {} index {}",
                        self.name, index
                    ));
                }
                update_color_image(&self.name, image, *actual_pixel, pixels, data, image_type)?;
                *actual_pixel += pixels;
                Ok(())
            }
            ImageSetMessage::End(pixels) => {
                self.sender.send_ack(self.id);
                let pixels = pixels as usize;
                let (mut image, actual_pixel) =
                    self.buffers.lock().remove(&index).ok_or_else(|| {
                        format!(
                            "No image buffer found for image collection: {} index {}",
                            self.name, index
                        )
                    })?;
                if actual_pixel + pixels != image.pixels.len() {
                    return Err(format!(
                        "Pixels do not match expected size in {} index {}: {} vs {}",
                        self.name,
                        index,
                        actual_pixel + pixels,
                        image.pixels.len()
                    ));
                }
                update_color_image(
                    &self.name,
                    &mut image,
                    actual_pixel,
                    pixels,
                    data,
                    image_type,
                )?;
                self.set_texture(index, image)
            }
        }
    }

    pub(crate) fn update_image(
        &self,
        index: u32,
        rect: [u32; 4],
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        self.sender.send_ack(self.id);
        let image_size = [rect[2] as usize, rect[3] as usize];
        let origin = [rect[0] as usize, rect[1] as usize];
        let image = create_color_image(&self.name, image_size, image_type, data)?;

        let mut inner = self.inner.write();
        let (texture, saved_size) = inner.images.get_mut(&index).ok_or_else(|| {
            format!(
                "Image index {} not found in collection: {}",
                index, self.name
            )
        })?;
        if *saved_size == image_size && origin == [0, 0] {
            texture.set(image, TEXTURE_OPTIONS);
        } else {
            if origin[0] + image_size[0] > saved_size[0]
                || origin[1] + image_size[1] > saved_size[1]
            {
                return Err(format!(
                    "Image is larger than the texture for image: {} index {}",
                    self.name, index
                ));
            }
            texture.set_partial(origin, image, TEXTURE_OPTIONS);
        }
        Ok(())
    }

    pub(crate) fn fill_image(
        &self,
        index: u32,
        size: [u32; 2],
        rgba: [u8; 4],
        data: &[u8],
    ) -> Result<(), String> {
        self.sender.send_ack(self.id);
        if !data.is_empty() {
            return Err(format!(
                "Fill message for image {} index {} contains unexpected pixel data",
                self.name, index
            ));
        }
        if size[0] == 0 || size[1] == 0 {
            return Err(format!(
                "Fill message for image {} index {} contains zero dimensions",
                self.name, index
            ));
        }
        let image_size = [size[0] as usize, size[1] as usize];
        image_size[0]
            .checked_mul(image_size[1])
            .ok_or_else(|| format!("Image dimensions overflow for image: {}", self.name))?;
        let [red, green, blue, alpha] = rgba;
        let pixel = egui::Color32::from_rgba_premultiplied(red, green, blue, alpha);
        self.set_texture(index, ColorImage::filled(image_size, pixel))
    }

    fn set_texture(&self, index: u32, image: ColorImage) -> Result<(), String> {
        let mut inner = self.inner.write();
        let size = image.size;
        if let Some((texture, saved_size)) = inner.images.get_mut(&index) {
            texture.set(image, TEXTURE_OPTIONS);
            *saved_size = size;
            return Ok(());
        }

        let context = inner.context.clone().ok_or_else(|| {
            format!(
                "ImageMulti must be initialized before receiving image: {}",
                self.name
            )
        })?;
        let name = format!("image_multi_{}_{}", self.id, index);
        let texture =
            context.load_texture(name, ImageData::Color(Arc::new(image)), TEXTURE_OPTIONS);
        inner.images.insert(index, (texture, size));
        Ok(())
    }

    pub(crate) fn remove(&self, index: u32) {
        self.inner.write().images.remove(&index);
        self.buffers.lock().remove(&index);
    }

    pub(crate) fn reset(&self) {
        self.inner.write().images.clear();
        self.buffers.lock().clear();
    }
}

impl Clone for ImageMulti {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            inner: self.inner.clone(),
            buffers: self.buffers.clone(),
            sender: self.sender.clone(),
        }
    }
}

fn update_color_image(
    name: &str,
    image: &mut ColorImage,
    actual_pixel: usize,
    pixels: usize,
    data: &[u8],
    image_type: ImageType,
) -> Result<(), String> {
    if actual_pixel + pixels > image.pixels.len() {
        return Err(format!("Pixels exceed image size in {name}"));
    }
    if image_type.bytes_per_pixel() * pixels != data.len() {
        return Err(format!(
            "Data length does not match expected size in {name}"
        ));
    }
    let data_ptr = data.as_ptr();
    let image_ptr = unsafe { image.pixels.as_mut_ptr().add(actual_pixel) as *mut u8 };
    unsafe { fill_c_image(image_type, data_ptr, image_ptr, pixels) }
    Ok(())
}

fn create_color_image(
    name: &str,
    image_size: [usize; 2],
    image_type: ImageType,
    data: &[u8],
) -> Result<ColorImage, String> {
    let expected = image_type
        .bytes_per_pixel()
        .checked_mul(image_size[0])
        .and_then(|value| value.checked_mul(image_size[1]))
        .ok_or_else(|| format!("Image dimensions overflow in {name}"))?;
    if expected != data.len() {
        return Err(format!(
            "Data length does not match expected size in {name}"
        ));
    }
    let mut image = ColorImage::filled(image_size, egui::Color32::TRANSPARENT);
    unsafe {
        fill_c_image(
            image_type,
            data.as_ptr(),
            image.pixels.as_mut_ptr() as *mut u8,
            image_size[0] * image_size[1],
        )
    }
    Ok(image)
}

/// Converts `pixel_count` source pixels into egui's four-byte pixel storage.
///
/// # Safety
///
/// `data_ptr` must point to at least `pixel_count * bytes_per_pixel` readable
/// bytes for `image_type`. `image_ptr` must point to at least
/// `pixel_count * 4` writable bytes, and the two regions must not overlap.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::messages::{ChannelMessage, MessageSender};
    use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

    fn assert_single_ack(
        receiver: &mut UnboundedReceiver<Option<ChannelMessage>>,
        expected_id: u64,
    ) {
        match receiver.try_recv() {
            Ok(Some(ChannelMessage::Ack(id))) => assert_eq!(id, expected_id),
            _ => panic!("expected an ACK for {expected_id}"),
        }
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn invalid_fill_messages_still_ack() {
        let id = 51;
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".to_string(), id, sender);

        assert!(image.fill_image([1, 1], [0, 0, 0, 0], &[1]).is_err());
        assert_single_ack(&mut receiver, id);

        assert!(image.fill_image([0, 1], [0, 0, 0, 0], &[]).is_err());
        assert_single_ack(&mut receiver, id);
    }

    #[test]
    fn valid_fill_keeps_its_existing_single_ack() {
        let id = 52;
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".to_string(), id, sender);

        image.fill_image([1, 1], [0, 0, 0, 0], &[]).unwrap();
        assert_single_ack(&mut receiver, id);
    }

    #[test]
    fn image_multi_creates_replaces_and_clears_sparse_textures() {
        let id = 53;
        let (sender, mut receiver) = MessageSender::new();
        let images = ImageMulti::new("images".to_string(), id, sender);

        assert!(
            images
                .set_image(
                    7,
                    ImageSetMessage::All([1, 1]),
                    ImageType::ColorAlpha,
                    &[1, 2, 3, 4],
                )
                .is_err()
        );
        assert_single_ack(&mut receiver, id);
        assert!(images.is_empty());

        images.initialize(&egui::Context::default());
        images.initialize(&egui::Context::default());
        images
            .set_image(7, ImageSetMessage::All([2, 1]), ImageType::Gray, &[10, 20])
            .unwrap();
        assert_single_ack(&mut receiver, id);
        images.fill_image(2, [1, 3], [1, 2, 3, 4], &[]).unwrap();
        assert_single_ack(&mut receiver, id);

        assert_eq!(images.len(), 2);
        assert_eq!(images.indices(), vec![2, 7]);
        assert!(images.contains(7));
        assert_eq!(images.get_size(7), Some([2, 1]));
        assert_eq!(images.get_size(2), Some([1, 3]));

        images.fill_image(7, [3, 2], [4, 3, 2, 1], &[]).unwrap();
        assert_single_ack(&mut receiver, id);
        assert_eq!(images.len(), 2);
        assert_eq!(images.get_size(7), Some([3, 2]));

        assert!(
            images
                .update_image(99, [0, 0, 1, 1], ImageType::Gray, &[5])
                .is_err()
        );
        assert_single_ack(&mut receiver, id);

        images.remove(2);
        assert_eq!(images.indices(), vec![7]);
        images.reset();
        assert!(images.is_empty());
    }

    #[test]
    fn image_multi_batch_buffers_are_independent_and_cleaned_up() {
        let id = 54;
        let (sender, mut receiver) = MessageSender::new();
        let images = ImageMulti::new("images".to_string(), id, sender);
        let context = egui::Context::default();
        images.initialize(&context);
        crate::test_support::texture_updates(&context);

        for index in [2, 7] {
            images
                .set_image(
                    index,
                    ImageSetMessage::Start([2, 1], 1),
                    ImageType::ColorAlpha,
                    &[index as u8, 0, 0, 255],
                )
                .unwrap();
        }
        assert!(images.is_empty(), "unfinished images must not publish");
        assert!(crate::test_support::texture_updates(&context).is_empty());
        assert!(receiver.try_recv().is_err());
        images
            .set_image(
                7,
                ImageSetMessage::End(1),
                ImageType::ColorAlpha,
                &[70, 0, 0, 255],
            )
            .unwrap();
        assert_single_ack(&mut receiver, id);
        assert_eq!(images.indices(), vec![7]);
        let delta = crate::test_support::texture_updates(&context);
        assert_eq!(delta.len(), 1);
        let egui::ImageData::Color(pixels) = &delta.values().next().unwrap()[0].image;
        assert_eq!(pixels.size, [2, 1]);
        assert_eq!(
            pixels
                .pixels
                .iter()
                .map(|p| p.to_array())
                .collect::<Vec<_>>(),
            [[7, 0, 0, 255], [70, 0, 0, 255]]
        );
        images
            .set_image(
                2,
                ImageSetMessage::End(1),
                ImageType::ColorAlpha,
                &[20, 0, 0, 255],
            )
            .unwrap();
        assert_single_ack(&mut receiver, id);
        assert_eq!(images.indices(), vec![2, 7]);
        let delta = crate::test_support::texture_updates(&context);
        assert_eq!(delta.len(), 1);
        let egui::ImageData::Color(pixels) = &delta.values().next().unwrap()[0].image;
        assert_eq!(pixels.size, [2, 1]);
        assert_eq!(
            pixels
                .pixels
                .iter()
                .map(|p| p.to_array())
                .collect::<Vec<_>>(),
            [[2, 0, 0, 255], [20, 0, 0, 255]]
        );

        images
            .set_image(11, ImageSetMessage::Start([2, 1], 1), ImageType::Gray, &[1])
            .unwrap();
        images.remove(11);
        assert!(
            images
                .set_image(11, ImageSetMessage::End(1), ImageType::Gray, &[2],)
                .is_err()
        );
        assert_single_ack(&mut receiver, id);

        images
            .set_image(12, ImageSetMessage::Start([2, 1], 1), ImageType::Gray, &[1])
            .unwrap();
        images.reset();
        assert!(images.buffers.lock().is_empty());
    }
}
