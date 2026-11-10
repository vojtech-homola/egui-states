//! Client image state backed by an egui texture.

use parking_lot::{Mutex, RwLock};
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;

use egui::{ColorImage, ImageData, TextureHandle};

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::hashing::NoHashMap;
use crate::image_transport::{ImageType, checked_image_bounds, checked_image_bytes};

/// A texture and its `[width, height]` size.
type Texture = (TextureHandle, [usize; 2]);

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
/// Each dimension is limited to 65,536 pixels, inclusive; renderer and memory
/// limits may be smaller.
///
/// The texture is created in the client's egui context when the first complete
/// image arrives from the server, or earlier with a placeholder from
/// [`Self::initialize`]; until then the getters return `None`. It uses
/// nearest-neighbor filtering and clamps sampling at its edges.
pub struct Image {
    name: Arc<String>,
    id: u64,
    context: egui::Context,
    sender: MessageSender,
    texture: Arc<RwLock<Option<Texture>>>,
    buffer: Arc<Mutex<Option<(ColorImage, usize)>>>,
}

impl Image {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        context: egui::Context,
    ) -> Self {
        Self {
            name: Arc::new(name),
            id,
            context,
            sender,
            texture: Arc::new(RwLock::new(None)),
            buffer: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the texture identifier and `[width, height]`, if the texture
    /// exists.
    pub fn get(&self) -> Option<(egui::TextureId, [usize; 2])> {
        self.texture
            .read()
            .as_ref()
            .map(|(texture_handle, size)| (texture_handle.id(), *size))
    }

    /// Returns the texture identifier, if the texture exists.
    pub fn get_id(&self) -> Option<egui::TextureId> {
        self.texture
            .read()
            .as_ref()
            .map(|(texture_handle, _)| texture_handle.id())
    }

    /// Returns the texture size as `[width, height]`, if the texture exists.
    pub fn get_size(&self) -> Option<[usize; 2]> {
        self.texture.read().as_ref().map(|(_, size)| *size)
    }

    /// Shows `image` as a placeholder until the server sends an image.
    ///
    /// Does nothing once the texture exists, so it never replaces an image
    /// from the server, whether it is called before or after connecting. The
    /// first image from the server replaces the placeholder. The getters then
    /// return `Some` even though the server has not sent an image yet.
    ///
    /// # Errors
    ///
    /// Returns an error if either image dimension is zero or exceeds 65,536
    /// pixels, or its RGBA byte count overflows. The renderer may impose a
    /// smaller size limit.
    pub fn initialize(&self, image: ColorImage) -> Result<(), String> {
        // Renderers such as wgpu panic on zero-sized textures.
        if image.size[0] == 0 || image.size[1] == 0 {
            return Err(format!(
                "Placeholder for image {} has zero dimensions",
                self.name
            ));
        }
        checked_image_bytes(image.size, 4)?;
        let mut texture = self.texture.write();
        if texture.is_none() {
            *texture = Some(self.create_texture(image));
        }
        Ok(())
    }

    /// Shows a complete image, creating the texture for the first one.
    fn set_texture(&self, image: ColorImage) {
        let mut texture = self.texture.write();
        match texture.as_mut() {
            Some((texture_handle, saved_size)) => {
                *saved_size = image.size;
                texture_handle.set(image, TEXTURE_OPTIONS);
            }
            None => *texture = Some(self.create_texture(image)),
        }
    }

    fn create_texture(&self, image: ColorImage) -> Texture {
        let size = image.size;
        let name = format!("image_{}", self.id);
        (load_texture(&self.context, name, image), size)
    }

    pub(crate) fn set_image(
        &self,
        message: ImageSetMessage,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        match message {
            ImageSetMessage::All(size) => {
                self.sender.send(ChannelMessage::Ack(self.id));
                let image_size = [size[0] as usize, size[1] as usize];
                let c_image = self.create_c_image(image_size, image_type, data)?;
                self.set_texture(c_image);
            }
            ImageSetMessage::Start(size, pixels) => {
                let pixels = pixels as usize;
                let size = [size[0] as usize, size[1] as usize];
                checked_image_bytes(size, 4)?;
                let mut c_image = ColorImage::filled(size, egui::Color32::WHITE);
                self.update_c_image(&mut c_image, 0, pixels, data, image_type)?;
                *self.buffer.lock() = Some((c_image, pixels))
            }
            ImageSetMessage::Batch(pixels) => {
                let pixels = pixels as usize;
                if let Some((ref mut c_image, ref mut actual_pixel)) = *self.buffer.lock() {
                    let actual = *actual_pixel;

                    if actual + pixels > c_image.pixels.len() {
                        return Err(format!("Pixels exceed image size in {}", self.name));
                    }

                    self.update_c_image(c_image, actual, pixels, data, image_type)?;
                    *actual_pixel += pixels;
                } else {
                    return Err(format!("No image buffer found for image: {}", self.name));
                }
            }
            ImageSetMessage::End(pixels) => {
                self.sender.send(ChannelMessage::Ack(self.id));
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
                    self.set_texture(c_image);
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
        self.sender.send(ChannelMessage::Ack(self.id));

        let image_size = [rect[2] as usize, rect[3] as usize];
        let origin = [rect[0] as usize, rect[1] as usize];

        checked_image_bounds(origin, image_size)?;
        let mut texture = self.texture.write();
        // A connection always starts with a complete image, so an update
        // without a texture is a protocol error.
        let Some((texture_handle, saved_size)) = texture.as_mut() else {
            return Err(format!(
                "Image {} received a partial update before a complete image",
                self.name
            ));
        };
        if origin[0] + image_size[0] > saved_size[0] || origin[1] + image_size[1] > saved_size[1] {
            return Err(format!(
                "Image is larger than the texture for image: {}",
                self.name
            ));
        }
        let c_image = self.create_c_image(image_size, image_type, data)?;
        if *saved_size == image_size && origin == [0, 0] {
            texture_handle.set(c_image, TEXTURE_OPTIONS);
        } else {
            texture_handle.set_partial(origin, c_image, TEXTURE_OPTIONS);
        }

        Ok(())
    }

    pub(crate) fn fill_image(
        &self,
        size: [u32; 2],
        rgba: [u8; 4],
        data: &[u8],
    ) -> Result<(), String> {
        self.sender.send_ack(self.id);
        if !data.is_empty() {
            return Err(format!(
                "Fill message for image {} contains unexpected pixel data",
                self.name
            ));
        }
        if size[0] == 0 || size[1] == 0 {
            return Err(format!(
                "Fill message for image {} contains zero dimensions",
                self.name
            ));
        }

        let image_size = [size[0] as usize, size[1] as usize];
        checked_image_bytes(image_size, 4)?;

        let [red, green, blue, alpha] = rgba;
        let pixel = egui::Color32::from_rgba_premultiplied(red, green, blue, alpha);
        self.set_texture(ColorImage::filled(image_size, pixel));

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
        create_color_image(&self.name, image_size, image_type, data)
    }
}

impl Clone for Image {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            context: self.context.clone(),
            sender: self.sender.clone(),
            texture: self.texture.clone(),
            buffer: self.buffer.clone(),
        }
    }
}

/// A server-controlled sparse collection of RGBA textures indexed by `u32` keys.
///
/// Each image dimension is limited to 65,536 pixels, inclusive; renderer and
/// memory limits may be smaller.
///
/// A complete image received for an absent key creates its texture in the
/// client's egui context. Textures use nearest-neighbor filtering and clamp
/// sampling at their edges.
pub struct ImageMulti {
    name: Arc<String>,
    id: u64,
    context: egui::Context,
    images: Arc<RwLock<NoHashMap<u32, Texture>>>,
    buffers: Arc<Mutex<NoHashMap<u32, (ColorImage, usize)>>>,
    sender: MessageSender,
}

impl ImageMulti {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        context: egui::Context,
    ) -> Self {
        Self {
            name: Arc::new(name),
            id,
            context,
            images: Arc::new(RwLock::new(NoHashMap::default())),
            buffers: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
        }
    }

    /// Returns the texture identifier and `[width, height]` for `index`, if
    /// populated.
    pub fn get(&self, index: u32) -> Option<(egui::TextureId, [usize; 2])> {
        self.images
            .read()
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
        self.images.read().len()
    }

    /// Returns whether the collection contains no images.
    pub fn is_empty(&self) -> bool {
        self.images.read().is_empty()
    }

    /// Returns whether `index` currently has an image.
    pub fn contains(&self, index: u32) -> bool {
        self.images.read().contains_key(&index)
    }

    /// Returns a snapshot of the populated indices in ascending order.
    pub fn indices(&self) -> Vec<u32> {
        let mut indices = self.images.read().keys().copied().collect::<Vec<_>>();
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
                checked_image_bytes(size, 4)?;
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
                if *actual_pixel + pixels > image.pixels.len() {
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
        checked_image_bounds(origin, image_size)?;

        let mut images = self.images.write();
        let (texture, saved_size) = images.get_mut(&index).ok_or_else(|| {
            format!(
                "Image index {} not found in collection: {}",
                index, self.name
            )
        })?;
        if origin[0] + image_size[0] > saved_size[0] || origin[1] + image_size[1] > saved_size[1] {
            return Err(format!(
                "Image is larger than the texture for image: {} index {}",
                self.name, index
            ));
        }
        let image = create_color_image(&self.name, image_size, image_type, data)?;
        if *saved_size == image_size && origin == [0, 0] {
            texture.set(image, TEXTURE_OPTIONS);
        } else {
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
        checked_image_bytes(image_size, 4)?;
        let [red, green, blue, alpha] = rgba;
        let pixel = egui::Color32::from_rgba_premultiplied(red, green, blue, alpha);
        self.set_texture(index, ColorImage::filled(image_size, pixel))
    }

    fn set_texture(&self, index: u32, image: ColorImage) -> Result<(), String> {
        checked_image_bytes(image.size, 4)?;
        let mut images = self.images.write();
        let size = image.size;
        if let Some((texture, saved_size)) = images.get_mut(&index) {
            texture.set(image, TEXTURE_OPTIONS);
            *saved_size = size;
            return Ok(());
        }

        let name = format!("image_multi_{}_{}", self.id, index);
        let texture = load_texture(&self.context, name, image);
        images.insert(index, (texture, size));
        Ok(())
    }

    pub(crate) fn remove(&self, index: u32) {
        self.images.write().remove(&index);
        self.buffers.lock().remove(&index);
    }

    pub(crate) fn reset(&self) {
        self.images.write().clear();
        self.buffers.lock().clear();
    }
}

impl Clone for ImageMulti {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            context: self.context.clone(),
            images: self.images.clone(),
            buffers: self.buffers.clone(),
            sender: self.sender.clone(),
        }
    }
}

/// Creates a texture for `image` in `context`.
///
/// This is [`egui::Context::load_texture`] without its debug assertion against
/// `max_texture_side`. Images arrive on the receive thread, possibly before the
/// first frame has reported the renderer's limit (egui assumes 2048 until
/// then), so the assertion could reject images the renderer accepts. Replacing
/// the image of an existing texture has no such assertion either.
fn load_texture(context: &egui::Context, name: String, image: ColorImage) -> TextureHandle {
    let manager = context.tex_manager();
    let id = manager
        .write()
        .alloc(name, ImageData::Color(Arc::new(image)), TEXTURE_OPTIONS);
    TextureHandle::new(manager, id)
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

pub(crate) fn create_color_image(
    name: &str,
    image_size: [usize; 2],
    image_type: ImageType,
    data: &[u8],
) -> Result<ColorImage, String> {
    checked_image_bytes(image_size, 4)?;
    let expected = checked_image_bytes(image_size, image_type.bytes_per_pixel())?;
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
    fn oversized_wire_images_preserve_textures_buffers_and_ack_behavior() {
        for received in [false, true] {
            let context = egui::Context::default();
            let (sender, mut receiver) = MessageSender::new();
            let image = Image::new("image".into(), 1, sender, context.clone());
            let (sender, mut multi_receiver) = MessageSender::new();
            let images = ImageMulti::new("images".into(), 2, sender, context.clone());
            if received {
                image.fill_image([1, 1], [0; 4], &[]).unwrap();
                assert_single_ack(&mut receiver, 1);
                images.fill_image(7, [1, 1], [0; 4], &[]).unwrap();
                assert_single_ack(&mut multi_receiver, 2);
            }
            let before = image.get();
            let multi_before = images.get(7);
            crate::test_support::texture_updates(&context);

            for size in [[65_537, 1], [1, 65_537]] {
                for operation in 0..4 {
                    let result = match operation {
                        0 => image.set_image(ImageSetMessage::All(size), ImageType::Gray, &[]),
                        1 => {
                            image.set_image(ImageSetMessage::Start(size, 1), ImageType::Gray, &[9])
                        }
                        2 => image.fill_image(size, [9; 4], &[]),
                        _ => image.update_image([0, 0, size[0], size[1]], ImageType::Gray, &[]),
                    };
                    assert!(result.unwrap_err().contains("65536"));
                    let result = match operation {
                        0 => images.set_image(7, ImageSetMessage::All(size), ImageType::Gray, &[]),
                        1 => images.set_image(
                            7,
                            ImageSetMessage::Start(size, 1),
                            ImageType::Gray,
                            &[9],
                        ),
                        2 => images.fill_image(7, size, [9; 4], &[]),
                        _ => images.update_image(7, [0, 0, size[0], size[1]], ImageType::Gray, &[]),
                    };
                    assert!(result.unwrap_err().contains("65536"));
                    if operation != 1 {
                        assert_single_ack(&mut receiver, 1);
                        assert_single_ack(&mut multi_receiver, 2);
                    } else {
                        assert!(receiver.try_recv().is_err());
                        assert!(multi_receiver.try_recv().is_err());
                    }
                    assert_eq!(image.get(), before);
                    assert_eq!(images.get(7), multi_before);
                    assert!(image.buffer.lock().is_none());
                    assert!(images.buffers.lock().is_empty());
                    assert!(crate::test_support::texture_updates(&context).is_empty());
                }
            }
            for origin in [[65_536, 0], [0, 65_536], [u32::MAX, 0], [0, u32::MAX]] {
                let rect = [origin[0], origin[1], 1, 1];
                assert!(image.update_image(rect, ImageType::Gray, &[9]).is_err());
                assert!(images.update_image(7, rect, ImageType::Gray, &[9]).is_err());
                assert_single_ack(&mut receiver, 1);
                assert_single_ack(&mut multi_receiver, 2);
            }
            assert_eq!(image.get(), before);
            assert_eq!(images.get(7), multi_before);
            assert!(crate::test_support::texture_updates(&context).is_empty());
        }
    }

    #[test]
    fn received_images_accept_the_inclusive_limit() {
        for size in [[65_536, 1], [1, 65_536]] {
            // The first image creates the texture before any frame has
            // reported the renderer's limit; egui assumes 2048 until then.
            let context = egui::Context::default();
            let (sender, mut receiver) = MessageSender::new();
            let image = Image::new("image".into(), 1, sender, context.clone());
            let (sender, mut multi_receiver) = MessageSender::new();
            let images = ImageMulti::new("images".into(), 2, sender, context.clone());

            let wire_size = [size[0] as u32, size[1] as u32];
            let data = vec![7; 65_536];
            image
                .set_image(ImageSetMessage::All(wire_size), ImageType::Gray, &data)
                .unwrap();
            images
                .set_image(7, ImageSetMessage::All(wire_size), ImageType::Gray, &data)
                .unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);

            image
                .set_image(ImageSetMessage::Start(wire_size, 1), ImageType::Gray, &[8])
                .unwrap();
            images
                .set_image(
                    7,
                    ImageSetMessage::Start(wire_size, 1),
                    ImageType::Gray,
                    &[8],
                )
                .unwrap();
            image
                .set_image(
                    ImageSetMessage::End(65_535),
                    ImageType::Gray,
                    &data[..65_535],
                )
                .unwrap();
            images
                .set_image(
                    7,
                    ImageSetMessage::End(65_535),
                    ImageType::Gray,
                    &data[..65_535],
                )
                .unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);

            image.fill_image(wire_size, [9; 4], &[]).unwrap();
            images.fill_image(7, wire_size, [9; 4], &[]).unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);
            let rect = [wire_size[0] - 1, wire_size[1] - 1, 1, 1];
            image.update_image(rect, ImageType::Gray, &[10]).unwrap();
            images
                .update_image(7, rect, ImageType::Gray, &[10])
                .unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);
            assert_eq!(image.get_size(), Some(size));
            assert_eq!(images.get_size(7), Some(size));
        }
    }

    /// A batch may complete the image, which leaves an empty final chunk.
    #[test]
    fn a_batch_may_complete_the_image_before_its_end() {
        let chunks = || {
            [
                (ImageSetMessage::Start([3, 1], 1), &[1][..]),
                (ImageSetMessage::Batch(2), &[2, 3][..]),
                (ImageSetMessage::End(0), &[][..]),
            ]
        };
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".into(), 1, sender, context.clone());
        let (sender, mut multi_receiver) = MessageSender::new();
        let images = ImageMulti::new("images".into(), 2, sender, context);

        for (message, data) in chunks() {
            image.set_image(message, ImageType::Gray, data).unwrap();
        }
        for (message, data) in chunks() {
            images.set_image(7, message, ImageType::Gray, data).unwrap();
        }
        assert_single_ack(&mut receiver, 1);
        assert_single_ack(&mut multi_receiver, 2);
        assert_eq!(image.get_size(), Some([3, 1]));
        assert_eq!(images.get_size(7), Some([3, 1]));
    }

    #[test]
    fn invalid_fill_messages_still_ack() {
        let id = 51;
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".to_string(), id, sender, egui::Context::default());

        assert!(image.fill_image([1, 1], [0, 0, 0, 0], &[1]).is_err());
        assert_single_ack(&mut receiver, id);

        assert!(image.fill_image([0, 1], [0, 0, 0, 0], &[]).is_err());
        assert_single_ack(&mut receiver, id);
    }

    #[test]
    fn valid_fill_keeps_its_existing_single_ack() {
        let id = 52;
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".to_string(), id, sender, egui::Context::default());

        image.fill_image([1, 1], [0, 0, 0, 0], &[]).unwrap();
        assert_single_ack(&mut receiver, id);
    }

    /// The first complete image creates the texture, whichever message carries
    /// it, and later images reuse that texture.
    #[test]
    fn the_first_complete_image_creates_the_texture() {
        let deliveries: [fn(&Image); 3] = [
            |image| {
                image
                    .set_image(ImageSetMessage::All([2, 1]), ImageType::Gray, &[1, 2])
                    .unwrap()
            },
            |image| {
                image
                    .set_image(ImageSetMessage::Start([2, 1], 1), ImageType::Gray, &[1])
                    .unwrap();
                image
                    .set_image(ImageSetMessage::End(1), ImageType::Gray, &[2])
                    .unwrap();
            },
            |image| image.fill_image([2, 1], [1, 2, 3, 255], &[]).unwrap(),
        ];
        for deliver in deliveries {
            let context = egui::Context::default();
            let (sender, mut receiver) = MessageSender::new();
            let image = Image::new("image".into(), 1, sender, context.clone());
            crate::test_support::texture_updates(&context);
            assert!(image.get().is_none());

            let error = image
                .update_image([0, 0, 1, 1], ImageType::Gray, &[9])
                .unwrap_err();
            assert!(error.contains("before a complete image"), "{error}");
            assert_single_ack(&mut receiver, 1);
            assert!(image.get().is_none());

            deliver(&image);
            assert_single_ack(&mut receiver, 1);
            let (id, size) = image.get().unwrap();
            assert_eq!(size, [2, 1]);
            let delta = crate::test_support::texture_updates(&context);
            assert_eq!(delta.len(), 1);
            assert_eq!(delta[&id].len(), 1);
            assert_eq!(delta[&id][0].image.size(), [2, 1]);

            image.fill_image([3, 2], [0; 4], &[]).unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_eq!(image.get(), Some((id, [3, 2])));
        }
    }

    /// `initialize` shows a placeholder only while no texture exists, so it
    /// never replaces an image from the server, whichever arrives first.
    #[test]
    fn initialize_sets_a_placeholder_only_without_a_texture() {
        let placeholder = |size| ColorImage::filled(size, egui::Color32::BLACK);
        let server_image = |image: &Image| {
            image
                .set_image(ImageSetMessage::All([3, 1]), ImageType::Gray, &[1, 2, 3])
                .unwrap()
        };

        // An empty or oversized placeholder creates nothing.
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".into(), 1, sender, context.clone());
        crate::test_support::texture_updates(&context);
        for size in [[0, 0], [0, 1], [1, 0]] {
            let error = image.initialize(placeholder(size)).unwrap_err();
            assert!(error.contains("zero dimensions"), "{error}");
        }
        let mut oversized = placeholder([1, 1]);
        oversized.size = [65_537, 1];
        assert!(image.initialize(oversized).unwrap_err().contains("65536"));
        assert!(image.get().is_none());
        assert!(crate::test_support::texture_updates(&context).is_empty());

        // Placeholder first: later calls keep it, the server's image replaces
        // it in the same texture.
        image.initialize(placeholder([2, 2])).unwrap();
        let (id, size) = image.get().unwrap();
        assert_eq!(size, [2, 2]);
        image.initialize(placeholder([4, 4])).unwrap();
        assert_eq!(image.get(), Some((id, [2, 2])));
        server_image(&image);
        assert_single_ack(&mut receiver, 1);
        assert_eq!(image.get(), Some((id, [3, 1])));

        // Server image first: the placeholder is ignored.
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::new("image".into(), 1, sender, context.clone());
        server_image(&image);
        assert_single_ack(&mut receiver, 1);
        let received = image.get();
        crate::test_support::texture_updates(&context);
        image.initialize(placeholder([2, 2])).unwrap();
        assert_eq!(image.get(), received);
        assert!(crate::test_support::texture_updates(&context).is_empty());
    }

    #[test]
    fn image_multi_creates_replaces_and_clears_sparse_textures() {
        let id = 53;
        let (sender, mut receiver) = MessageSender::new();
        let images = ImageMulti::new("images".to_string(), id, sender, egui::Context::default());

        // The first image for a key creates its texture without any setup.
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
        let context = egui::Context::default();
        let images = ImageMulti::new("images".to_string(), id, sender, context.clone());
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
