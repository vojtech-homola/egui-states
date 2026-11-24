//! Client image state backed by an egui texture.

use parking_lot::{Mutex, RwLock};
use std::fmt::{self, Display};
use std::sync::Arc;

use egui::{ColorImage, ImageData, TextureHandle};

use crate::client::messages::{ChannelMessage, MessageSender};
use crate::client::states::header::{
    HeaderValue, Label, created_header, decode_header, decode_header_bytes, header_type_id,
    split_header,
};
use crate::hashing::NoHashMap;
use crate::protocol::data::{HeaderBytes, HeaderUpdate};
use crate::protocol::image::{
    ImageHeader, ImageSetHeader, ImageType, ImageUpdateHeader, checked_image_bounds,
    checked_image_bytes, to_rgba,
};

/// A texture and its `[width, height]` size.
type Texture = (TextureHandle, [usize; 2]);

/// The texture of an [`Image`] and its header, which is `None` only for a
/// placeholder without one.
type Shown<H> = (Texture, Option<H>);

/// How a texture is sampled until `set_texture_options` changes it.
const DEFAULT_TEXTURE_OPTIONS: egui::TextureOptions = egui::TextureOptions::NEAREST;

/// The receive task's side of an [`Image`] of any header type.
pub(crate) trait UpdateImage: Send + Sync {
    /// Applies one image message, whose payload holds its header bytes and
    /// then its pixels.
    fn apply(&self, header: ImageHeader, payload: &[u8]) -> Result<(), String>;
    /// Drops a partial image transfer of a closed connection.
    fn disconnected(&self);
}

/// The receive task's side of an [`ImageMulti`] of any header type.
pub(crate) trait UpdateImageMulti: Send + Sync {
    /// Applies one image message of `index`, whose payload holds its header
    /// bytes and then its pixels.
    fn apply(&self, index: u32, header: ImageHeader, payload: &[u8]) -> Result<(), String>;
    fn remove(&self, index: u32);
    fn reset(&self);
    /// Drops every partial image transfer of a closed connection.
    fn disconnected(&self);
}

/// A server-controlled RGBA texture used by the egui client, with an
/// optional typed header `H` that the server sends with the image.
///
/// Each dimension is limited to 65,536 pixels, inclusive; renderer and memory
/// limits may be smaller.
///
/// The texture is created in the client's egui context when the first complete
/// image arrives from the server, or earlier with a placeholder from
/// [`Self::initialize`]; until then the getters return `None`. By default it
/// uses nearest-neighbor filtering and clamps sampling at its edges; see
/// [`Self::set_texture_options`].
///
/// A header that arrives with an image or an update is stored together with
/// the pixels it describes, so a reader never sees the new header with the
/// old pixels. egui uploads a texture change at the end of the frame that is
/// being built, though, so for that one frame the UI can still draw the old
/// pixels next to a new header, or the new pixels next to a header read
/// before they arrived.
pub struct Image<H = ()> {
    name: Arc<String>,
    id: u64,
    header_type: Option<u32>,
    context: egui::Context,
    sender: MessageSender,
    texture: Arc<RwLock<Option<Shown<H>>>>,
    buffer: Arc<Mutex<Option<PartialImage>>>,
    options: Arc<Mutex<egui::TextureOptions>>,
}

impl<H: HeaderValue> Image<H> {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        context: egui::Context,
    ) -> Self {
        Self {
            name: Arc::new(name),
            id,
            header_type: header_type_id::<H>(),
            context,
            sender,
            texture: Arc::new(RwLock::new(None)),
            buffer: Arc::new(Mutex::new(None)),
            options: Arc::new(Mutex::new(DEFAULT_TEXTURE_OPTIONS)),
        }
    }

    /// Sets how the texture is sampled when it is drawn larger or smaller
    /// than its pixels, and at its edges. The default is
    /// [`egui::TextureOptions::NEAREST`]: every screen pixel shows the
    /// nearest image pixel, so zoomed-in pixels stay sharp, but a large image
    /// drawn much smaller skips pixels. [`egui::TextureOptions::LINEAR`]
    /// blends neighboring pixels instead.
    ///
    /// The options apply from the next image the server sends, a complete
    /// one or an update, and to a placeholder created afterwards; until
    /// then an existing texture keeps its options, since the client keeps
    /// no copy of the pixels to upload again. Call it before connecting to
    /// cover the first image. The options are shared by every clone of this
    /// handle.
    ///
    /// egui's `wgpu` renderer, eframe's default, ignores
    /// [`egui::TextureOptions::mipmap_mode`]; the `glow` renderer generates
    /// the mipmaps.
    pub fn set_texture_options(&self, options: egui::TextureOptions) {
        *self.options.lock() = options;
    }

    /// Returns the options the next texture upload uses; see
    /// [`Self::set_texture_options`].
    pub fn texture_options(&self) -> egui::TextureOptions {
        *self.options.lock()
    }

    /// Returns the texture identifier and `[width, height]`, if the texture
    /// exists.
    pub fn get(&self) -> Option<(egui::TextureId, [usize; 2])> {
        self.texture
            .read()
            .as_ref()
            .map(|((texture_handle, size), _)| (texture_handle.id(), *size))
    }

    /// Returns the texture identifier, if the texture exists.
    pub fn get_id(&self) -> Option<egui::TextureId> {
        self.get().map(|(id, _)| id)
    }

    /// Returns the texture size as `[width, height]`, if the texture exists.
    pub fn get_size(&self) -> Option<[usize; 2]> {
        self.get().map(|(_, size)| size)
    }

    /// Returns a clone of the header, `None` until the texture exists, and
    /// for a placeholder from [`Self::initialize`] until the server sends
    /// an image.
    pub fn header(&self) -> Option<H> {
        self.read_header(|header| header.cloned())
    }

    /// Calls `f` with a reference to the header, `None` when
    /// [`Self::header`] is.
    pub fn read_header<R>(&self, f: impl FnOnce(Option<&H>) -> R) -> R {
        f(self
            .texture
            .read()
            .as_ref()
            .and_then(|(_, header)| header.as_ref()))
    }

    /// Returns the texture identifier, its `[width, height]` and a clone of
    /// its header, read together; `None` when [`Self::header`] is.
    pub fn get_with_header(&self) -> Option<(egui::TextureId, [usize; 2], H)> {
        let texture = self.texture.read();
        let ((texture_handle, size), header) = texture.as_ref()?;
        Some((texture_handle.id(), *size, header.clone()?))
    }

    /// Shows `image` as a placeholder until the server sends an image.
    ///
    /// Does nothing once the texture exists, so it never replaces an image
    /// from the server, whether it is called before or after connecting. The
    /// first image from the server replaces the placeholder. The getters then
    /// return `Some` even though the server has not sent an image yet. An
    /// image with a header type has no header until the server sends one;
    /// [`Self::initialize_with_header`] gives the placeholder one.
    ///
    /// # Errors
    ///
    /// Returns an error if either image dimension is zero or exceeds 65,536
    /// pixels, or its RGBA byte count overflows. The renderer may impose a
    /// smaller size limit.
    pub fn initialize(&self, image: ColorImage) -> Result<(), String> {
        // Without a header type, the header is the value no bytes encode.
        let header = created_header(self.header_type, None, self.label()).ok();
        self.place(image, header)
    }

    /// Like [`Self::initialize`], with `header` as the placeholder's header,
    /// which the first image from the server replaces too.
    ///
    /// # Errors
    ///
    /// As [`Self::initialize`].
    pub fn initialize_with_header(&self, image: ColorImage, header: H) -> Result<(), String> {
        self.place(image, Some(header))
    }

    fn place(&self, image: ColorImage, header: Option<H>) -> Result<(), String> {
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
            *texture = Some((self.create_texture(image), header));
        }
        Ok(())
    }

    /// Names the state in header errors.
    fn label(&self) -> Label<'_> {
        Label("Image", &self.name)
    }

    /// Decodes the header update of a message that completes a change and
    /// splits it off the payload, leaving the pixels.
    fn split<'a>(
        &self,
        update: HeaderUpdate,
        payload: &'a [u8],
    ) -> Result<(Option<H>, &'a [u8]), String> {
        let (header, pixels) = split_header(payload, update.len(), self.label())?;
        let header = decode_header(self.header_type, update, header, self.label())?;
        Ok((header, pixels))
    }

    /// Shows a complete image, creating the texture for the first one. A
    /// given header replaces the current one.
    fn set_texture(&self, image: ColorImage, header: Option<H>) -> Result<(), String> {
        let mut texture = self.texture.write();
        let has_header = texture.as_ref().is_some_and(|(_, header)| header.is_some());
        let header = match header {
            None if !has_header => Some(created_header(self.header_type, None, self.label())?),
            header => header,
        };
        match texture.as_mut() {
            Some(((texture_handle, saved_size), current)) => {
                *saved_size = image.size;
                texture_handle.set(image, self.texture_options());
                if header.is_some() {
                    *current = header;
                }
            }
            None => *texture = Some((self.create_texture(image), header)),
        }
        Ok(())
    }

    fn create_texture(&self, image: ColorImage) -> Texture {
        let size = image.size;
        let name = format!("image_{}", self.id);
        let options = self.texture_options();
        (load_texture(&self.context, name, image, options), size)
    }

    pub(crate) fn set_image(
        &self,
        message: ImageSetHeader,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        match message {
            ImageSetHeader::All(size, update) => {
                self.sender.send(ChannelMessage::Ack(self.id));
                let (header, data) = self.split(update, data)?;
                let image_size = [size[0] as usize, size[1] as usize];
                let c_image = create_color_image(&self.name, image_size, image_type, data)?;
                self.set_texture(c_image, header)?;
            }
            ImageSetHeader::Start(size, pixels) => {
                let partial = PartialImage::start(&self.name, size, pixels, image_type, data)?;
                *self.buffer.lock() = Some(partial)
            }
            ImageSetHeader::Batch(pixels) => match *self.buffer.lock() {
                Some(ref mut partial) if partial.origin.is_none() => {
                    partial.append(&self.name, pixels, image_type, data)?
                }
                _ => return Err(format!("No image buffer found for image: {}", self.name)),
            },
            ImageSetHeader::End(pixels, update) => {
                self.sender.send(ChannelMessage::Ack(self.id));
                let Some(partial) = self.buffer.lock().take().filter(|p| p.origin.is_none()) else {
                    return Err(format!("No image buffer found for image: {}", self.name));
                };
                let (header, data) = self.split(update, data)?;
                let c_image = partial.finish(&self.name, pixels, image_type, data)?;
                self.set_texture(c_image, header)?;
            }
        }

        Ok(())
    }

    /// Applies one message of a rectangle update; the rectangle is applied
    /// once, at `All` or `End`, which are acknowledged also when they fail.
    pub(crate) fn update_image(
        &self,
        header: ImageUpdateHeader,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        match header {
            ImageUpdateHeader::All(rect, update) => {
                self.sender.send(ChannelMessage::Ack(self.id));
                let (origin, size) = self.checked_update(rect)?;
                let (header, data) = self.split(update, data)?;
                let image = create_color_image(&self.name, size, image_type, data)?;
                self.put_texture_update(origin, image, header)
            }
            ImageUpdateHeader::Start(rect, pixels) => {
                let (origin, _) = self.checked_update(rect)?;
                let partial =
                    PartialImage::start(&self.name, [rect[2], rect[3]], pixels, image_type, data)?;
                *self.buffer.lock() = Some(partial.at(origin));
                Ok(())
            }
            ImageUpdateHeader::Batch(pixels) => match *self.buffer.lock() {
                Some(ref mut partial) if partial.origin.is_some() => {
                    partial.append(&self.name, pixels, image_type, data)
                }
                _ => Err(self.no_update_buffer()),
            },
            ImageUpdateHeader::End(pixels, update) => {
                self.sender.send(ChannelMessage::Ack(self.id));
                let partial = self.buffer.lock().take();
                let Some((partial, origin)) =
                    partial.and_then(|p| p.origin.map(|origin| (p, origin)))
                else {
                    return Err(self.no_update_buffer());
                };
                let (header, data) = self.split(update, data)?;
                let image = partial.finish(&self.name, pixels, image_type, data)?;
                self.put_texture_update(origin, image, header)
            }
        }
    }

    fn no_update_buffer(&self) -> String {
        format!("No image update buffer found for image: {}", self.name)
    }

    fn no_texture(&self) -> String {
        format!(
            "Image {} received a partial update before a complete image",
            self.name
        )
    }

    /// Checks a wire rectangle against the texture and returns its origin and
    /// size as `[x, y]` and `[width, height]`.
    fn checked_update(&self, rect: [u32; 4]) -> Result<([usize; 2], [usize; 2]), String> {
        let size = [rect[2] as usize, rect[3] as usize];
        let origin = [rect[0] as usize, rect[1] as usize];
        checked_image_bounds(origin, size)?;
        // A connection always starts with a complete image, so an update
        // without a texture is a protocol error. Only this receive task
        // changes the texture, so it stays as checked while the pixels are
        // converted outside the lock, which UI reads would wait for.
        let texture_size = self.get_size().ok_or_else(|| self.no_texture())?;
        check_update(texture_size, &self.name, origin, size)?;
        Ok((origin, size))
    }

    fn put_texture_update(
        &self,
        origin: [usize; 2],
        image: ColorImage,
        header: Option<H>,
    ) -> Result<(), String> {
        let mut texture = self.texture.write();
        let ((texture_handle, saved_size), current) =
            texture.as_mut().ok_or_else(|| self.no_texture())?;
        check_update(*saved_size, &self.name, origin, image.size)?;
        put_update(
            texture_handle,
            *saved_size,
            origin,
            image,
            self.texture_options(),
        );
        if header.is_some() {
            *current = header;
        }
        Ok(())
    }

    pub(crate) fn fill_image(
        &self,
        size: [u32; 2],
        rgba: [u8; 4],
        update: HeaderUpdate,
        data: &[u8],
    ) -> Result<(), String> {
        self.sender.send_ack(self.id);
        let (header, data) = self.split(update, data)?;
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
        self.set_texture(ColorImage::filled(image_size, pixel), header)
    }

    /// Replaces the header alone; the image must exist.
    fn replace_header(&self, bytes: HeaderBytes, data: &[u8]) -> Result<(), String> {
        let header = decode_header_bytes(self.header_type, bytes, data, self.label())?;
        let mut texture = self.texture.write();
        let (_, current) = texture
            .as_mut()
            .ok_or_else(|| format!("Image {} received a header before an image", self.name))?;
        *current = Some(header);
        Ok(())
    }
}

impl<H: HeaderValue> UpdateImage for Image<H> {
    fn apply(&self, header: ImageHeader, payload: &[u8]) -> Result<(), String> {
        match header {
            ImageHeader::Set(set, image_type) => self.set_image(set, image_type, payload),
            ImageHeader::Update(update, image_type) => {
                self.update_image(update, image_type, payload)
            }
            ImageHeader::Fill(size, rgba, update) => self.fill_image(size, rgba, update, payload),
            ImageHeader::Header(bytes) => self.replace_header(bytes, payload),
        }
    }

    fn disconnected(&self) {
        self.buffer.lock().take();
    }
}

impl<H> Clone for Image<H> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            header_type: self.header_type,
            context: self.context.clone(),
            sender: self.sender.clone(),
            texture: self.texture.clone(),
            buffer: self.buffer.clone(),
            options: self.options.clone(),
        }
    }
}

/// A server-controlled sparse collection of RGBA textures indexed by `u32`
/// keys, each with an optional typed header `H` that the server sends with
/// its image.
///
/// Each image dimension is limited to 65,536 pixels, inclusive; renderer and
/// memory limits may be smaller.
///
/// A complete image received for an absent key creates its texture in the
/// client's egui context. By default textures use nearest-neighbor filtering
/// and clamp sampling at their edges; see [`Self::set_texture_options`]. A key's header is stored together with its
/// pixels, as for [`Image`], including the one frame egui may show the old
/// pixels with a new header.
pub struct ImageMulti<H = ()> {
    name: Arc<String>,
    id: u64,
    header_type: Option<u32>,
    context: egui::Context,
    images: Arc<RwLock<NoHashMap<u32, (Texture, H)>>>,
    buffers: Arc<Mutex<NoHashMap<u32, PartialImage>>>,
    sender: MessageSender,
    options: Arc<Mutex<egui::TextureOptions>>,
}

impl<H: HeaderValue> ImageMulti<H> {
    pub(crate) fn new(
        name: String,
        id: u64,
        sender: MessageSender,
        context: egui::Context,
    ) -> Self {
        Self {
            name: Arc::new(name),
            id,
            header_type: header_type_id::<H>(),
            context,
            images: Arc::new(RwLock::new(NoHashMap::default())),
            buffers: Arc::new(Mutex::new(NoHashMap::default())),
            sender,
            options: Arc::new(Mutex::new(DEFAULT_TEXTURE_OPTIONS)),
        }
    }

    /// Sets how the textures of every key are sampled, as
    /// [`Image::set_texture_options`] does for one image.
    ///
    /// The options apply to each key from the next image the server sends
    /// for it, a complete one or an update, and to every key created
    /// afterwards; until then a key keeps its options. Call it before
    /// connecting to cover the first images.
    pub fn set_texture_options(&self, options: egui::TextureOptions) {
        *self.options.lock() = options;
    }

    /// Returns the options the next texture upload uses; see
    /// [`Self::set_texture_options`].
    pub fn texture_options(&self) -> egui::TextureOptions {
        *self.options.lock()
    }

    /// Returns the texture identifier and `[width, height]` for `index`, if
    /// populated.
    pub fn get(&self, index: u32) -> Option<(egui::TextureId, [usize; 2])> {
        self.images
            .read()
            .get(&index)
            .map(|((texture, size), _)| (texture.id(), *size))
    }

    /// Returns the texture identifier for `index`, if populated.
    pub fn get_id(&self, index: u32) -> Option<egui::TextureId> {
        self.get(index).map(|(id, _)| id)
    }

    /// Returns the `[width, height]` texture size for `index`, if populated.
    pub fn get_size(&self, index: u32) -> Option<[usize; 2]> {
        self.get(index).map(|(_, size)| size)
    }

    /// Returns a clone of the header of `index`, if populated.
    pub fn header(&self, index: u32) -> Option<H> {
        self.read_header(index, |header| header.cloned())
    }

    /// Calls `f` with a reference to the header of `index`, `None` if
    /// unpopulated.
    pub fn read_header<R>(&self, index: u32, f: impl FnOnce(Option<&H>) -> R) -> R {
        f(self.images.read().get(&index).map(|(_, header)| header))
    }

    /// Returns the texture identifier, `[width, height]` and a clone of the
    /// header of `index`, read together, if populated.
    pub fn get_with_header(&self, index: u32) -> Option<(egui::TextureId, [usize; 2], H)> {
        self.images
            .read()
            .get(&index)
            .map(|((texture, size), header)| (texture.id(), *size, header.clone()))
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

    /// Decodes the header update of a message of `index` that completes a
    /// change and splits it off the payload, leaving the pixels.
    fn split<'a>(
        &self,
        index: u32,
        update: HeaderUpdate,
        payload: &'a [u8],
    ) -> Result<(Option<H>, &'a [u8]), String> {
        let label = self.label(index);
        let (header, pixels) = split_header(payload, update.len(), label)?;
        let header = decode_header(self.header_type, update, header, label)?;
        Ok((header, pixels))
    }

    pub(crate) fn set_image(
        &self,
        index: u32,
        message: ImageSetHeader,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        match message {
            ImageSetHeader::All(size, update) => {
                self.sender.send_ack_key(self.id, index);
                let (header, data) = self.split(index, update, data)?;
                let image_size = [size[0] as usize, size[1] as usize];
                let image = create_color_image(&self.label(index), image_size, image_type, data)?;
                self.set_texture(index, image, header)
            }
            ImageSetHeader::Start(size, pixels) => {
                let label = self.label(index);
                let partial = PartialImage::start(&label, size, pixels, image_type, data)?;
                self.buffers.lock().insert(index, partial);
                Ok(())
            }
            ImageSetHeader::Batch(pixels) => {
                let mut buffers = self.buffers.lock();
                let partial = buffers
                    .get_mut(&index)
                    .filter(|partial| partial.origin.is_none())
                    .ok_or_else(|| self.no_buffer(index))?;
                partial.append(&self.label(index), pixels, image_type, data)
            }
            ImageSetHeader::End(pixels, update) => {
                self.sender.send_ack_key(self.id, index);
                let partial = self
                    .buffers
                    .lock()
                    .remove(&index)
                    .filter(|partial| partial.origin.is_none())
                    .ok_or_else(|| self.no_buffer(index))?;
                let (header, data) = self.split(index, update, data)?;
                let image = partial.finish(&self.label(index), pixels, image_type, data)?;
                self.set_texture(index, image, header)
            }
        }
    }

    /// Applies one message of a rectangle update of `index`; the rectangle
    /// is applied once, at `All` or `End`, which are acknowledged also when
    /// they fail.
    pub(crate) fn update_image(
        &self,
        index: u32,
        header: ImageUpdateHeader,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        let label = self.label(index);
        match header {
            ImageUpdateHeader::All(rect, update) => {
                self.sender.send_ack_key(self.id, index);
                let (origin, size) = self.checked_update(index, rect)?;
                let (header, data) = self.split(index, update, data)?;
                // Convert outside the map lock, which UI reads of every key
                // wait for.
                let image = create_color_image(&label, size, image_type, data)?;
                self.put_texture_update(index, origin, image, header)
            }
            ImageUpdateHeader::Start(rect, pixels) => {
                let (origin, _) = self.checked_update(index, rect)?;
                let partial =
                    PartialImage::start(&label, [rect[2], rect[3]], pixels, image_type, data)?;
                self.buffers.lock().insert(index, partial.at(origin));
                Ok(())
            }
            ImageUpdateHeader::Batch(pixels) => {
                let mut buffers = self.buffers.lock();
                let partial = buffers
                    .get_mut(&index)
                    .filter(|partial| partial.origin.is_some())
                    .ok_or_else(|| self.no_buffer(index))?;
                partial.append(&label, pixels, image_type, data)
            }
            ImageUpdateHeader::End(pixels, update) => {
                self.sender.send_ack_key(self.id, index);
                let partial = self.buffers.lock().remove(&index);
                let (partial, origin) = partial
                    .and_then(|p| p.origin.map(|origin| (p, origin)))
                    .ok_or_else(|| self.no_buffer(index))?;
                let (header, data) = self.split(index, update, data)?;
                let image = partial.finish(&label, pixels, image_type, data)?;
                self.put_texture_update(index, origin, image, header)
            }
        }
    }

    fn not_found(&self, index: u32) -> String {
        format!(
            "Image index {} not found in collection: {}",
            index, self.name
        )
    }

    /// Checks a wire rectangle against the texture of `index` and returns its
    /// origin and size as `[x, y]` and `[width, height]`.
    fn checked_update(
        &self,
        index: u32,
        rect: [u32; 4],
    ) -> Result<([usize; 2], [usize; 2]), String> {
        let size = [rect[2] as usize, rect[3] as usize];
        let origin = [rect[0] as usize, rect[1] as usize];
        checked_image_bounds(origin, size)?;
        let texture_size = self.get_size(index).ok_or_else(|| self.not_found(index))?;
        check_update(texture_size, &self.label(index), origin, size)?;
        Ok((origin, size))
    }

    fn put_texture_update(
        &self,
        index: u32,
        origin: [usize; 2],
        image: ColorImage,
        header: Option<H>,
    ) -> Result<(), String> {
        let mut images = self.images.write();
        let ((texture, saved_size), current) = images
            .get_mut(&index)
            .ok_or_else(|| self.not_found(index))?;
        check_update(*saved_size, &self.label(index), origin, image.size)?;
        put_update(texture, *saved_size, origin, image, self.texture_options());
        if let Some(header) = header {
            *current = header;
        }
        Ok(())
    }

    /// Names one image of the collection in errors.
    fn label(&self, index: u32) -> KeyLabel<'_> {
        KeyLabel(&self.name, index)
    }

    fn no_buffer(&self, index: u32) -> String {
        format!(
            "No image buffer found for image collection: {} index {}",
            self.name, index
        )
    }

    pub(crate) fn fill_image(
        &self,
        index: u32,
        size: [u32; 2],
        rgba: [u8; 4],
        update: HeaderUpdate,
        data: &[u8],
    ) -> Result<(), String> {
        self.sender.send_ack_key(self.id, index);
        let (header, data) = self.split(index, update, data)?;
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
        self.set_texture(index, ColorImage::filled(image_size, pixel), header)
    }

    /// Shows a complete image at `index`, creating its texture for a new
    /// key, which needs a header if the state has a header type. A given
    /// header replaces the current one.
    fn set_texture(&self, index: u32, image: ColorImage, header: Option<H>) -> Result<(), String> {
        checked_image_bytes(image.size, 4)?;
        let mut images = self.images.write();
        let size = image.size;
        let options = self.texture_options();
        if let Some(((texture, saved_size), current)) = images.get_mut(&index) {
            texture.set(image, options);
            *saved_size = size;
            if let Some(header) = header {
                *current = header;
            }
            return Ok(());
        }

        let header = created_header(self.header_type, header, self.label(index))?;
        let name = format!("image_multi_{}_{}", self.id, index);
        let texture = load_texture(&self.context, name, image, options);
        images.insert(index, ((texture, size), header));
        Ok(())
    }

    /// Replaces the header of `index` alone; the image must exist.
    fn replace_header(&self, index: u32, bytes: HeaderBytes, data: &[u8]) -> Result<(), String> {
        let header = decode_header_bytes(self.header_type, bytes, data, self.label(index))?;
        let mut images = self.images.write();
        let (_, current) = images
            .get_mut(&index)
            .ok_or_else(|| self.not_found(index))?;
        *current = header;
        Ok(())
    }
}

impl<H: HeaderValue> UpdateImageMulti for ImageMulti<H> {
    fn apply(&self, index: u32, header: ImageHeader, payload: &[u8]) -> Result<(), String> {
        match header {
            ImageHeader::Set(set, image_type) => self.set_image(index, set, image_type, payload),
            ImageHeader::Update(update, image_type) => {
                self.update_image(index, update, image_type, payload)
            }
            ImageHeader::Fill(size, rgba, update) => {
                self.fill_image(index, size, rgba, update, payload)
            }
            ImageHeader::Header(bytes) => self.replace_header(index, bytes, payload),
        }
    }

    fn remove(&self, index: u32) {
        self.images.write().remove(&index);
        self.buffers.lock().remove(&index);
    }

    fn reset(&self) {
        self.images.write().clear();
        self.buffers.lock().clear();
    }

    fn disconnected(&self) {
        self.buffers.lock().clear();
    }
}

impl<H> Clone for ImageMulti<H> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            header_type: self.header_type,
            context: self.context.clone(),
            images: self.images.clone(),
            buffers: self.buffers.clone(),
            sender: self.sender.clone(),
            options: self.options.clone(),
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
fn load_texture(
    context: &egui::Context,
    name: String,
    image: ColorImage,
    options: egui::TextureOptions,
) -> TextureHandle {
    let manager = context.tex_manager();
    let id = manager
        .write()
        .alloc(name, ImageData::Color(Arc::new(image)), options);
    TextureHandle::new(manager, id)
}

/// Names one image of an image collection in errors; it is formatted only
/// when one occurs, not for every chunk.
#[derive(Clone, Copy)]
struct KeyLabel<'a>(&'a str, u32);

impl Display for KeyLabel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} index {}", self.0, self.1)
    }
}

/// An image set or rectangle update being assembled from its chunks;
/// `pixels` holds the pixels received so far. `origin` is the `[x, y]` of an
/// update's rectangle and `None` for a whole-image set.
struct PartialImage {
    size: [usize; 2],
    origin: Option<[usize; 2]>,
    pixels: Vec<egui::Color32>,
}

impl PartialImage {
    fn start(
        label: &dyn Display,
        size: [u32; 2],
        pixels: u32,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<Self, String> {
        let size = [size[0] as usize, size[1] as usize];
        checked_image_bytes(size, 4)?;
        let mut image = Self {
            size,
            origin: None,
            pixels: Vec::with_capacity(size[0] * size[1]),
        };
        image.append(label, pixels, image_type, data)?;
        Ok(image)
    }

    /// Marks the transfer as a rectangle update at `origin`.
    fn at(mut self, origin: [usize; 2]) -> Self {
        self.origin = Some(origin);
        self
    }

    fn total(&self) -> usize {
        self.size[0] * self.size[1]
    }

    fn append(
        &mut self,
        label: &dyn Display,
        pixels: u32,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<(), String> {
        let pixels = pixels as usize;
        if self.pixels.len() + pixels > self.total() {
            return Err(format!("Pixels exceed image size in {label}"));
        }
        convert_pixels(label, &mut self.pixels, pixels, image_type, data)
    }

    fn finish(
        mut self,
        label: &dyn Display,
        pixels: u32,
        image_type: ImageType,
        data: &[u8],
    ) -> Result<ColorImage, String> {
        let received = self.pixels.len() + pixels as usize;
        if received != self.total() {
            return Err(format!(
                "Pixels do not match expected size in {label}: {received} vs {}",
                self.total()
            ));
        }
        convert_pixels(label, &mut self.pixels, pixels as usize, image_type, data)?;
        Ok(ColorImage::new(self.size, self.pixels))
    }
}

/// Validates an update at `origin` against a texture of `texture_size`.
fn check_update(
    texture_size: [usize; 2],
    label: &dyn Display,
    origin: [usize; 2],
    size: [usize; 2],
) -> Result<(), String> {
    if origin[0] + size[0] > texture_size[0] || origin[1] + size[1] > texture_size[1] {
        return Err(format!(
            "Image is larger than the texture for image: {label}"
        ));
    }
    Ok(())
}

/// Applies a converted, checked update at `origin`; an update of the whole
/// texture replaces it.
fn put_update(
    texture: &mut TextureHandle,
    texture_size: [usize; 2],
    origin: [usize; 2],
    image: ColorImage,
    options: egui::TextureOptions,
) {
    if texture_size == image.size && origin == [0, 0] {
        texture.set(image, options);
    } else {
        texture.set_partial(origin, image, options);
    }
}

/// Converts `count` source pixels from `data` and appends them to `pixels`.
fn convert_pixels(
    label: &dyn Display,
    pixels: &mut Vec<egui::Color32>,
    count: usize,
    image_type: ImageType,
    data: &[u8],
) -> Result<(), String> {
    if image_type.bytes_per_pixel() * count != data.len() {
        return Err(format!(
            "Data length does not match expected size in {label}"
        ));
    }
    pixels.reserve(count);
    // SAFETY: `data` holds `count` source pixels, `reserve` made room for
    // `count` four-byte `Color32` values, and the regions cannot overlap;
    // `fill_c_image` writes every one of them.
    unsafe {
        fill_c_image(
            image_type,
            data.as_ptr(),
            pixels.as_mut_ptr().add(pixels.len()).cast::<u8>(),
            count,
        );
        pixels.set_len(pixels.len() + count);
    }
    Ok(())
}

pub(crate) fn create_color_image(
    name: &dyn Display,
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
    let count = image_size[0] * image_size[1];
    let mut pixels = Vec::with_capacity(count);
    convert_pixels(name, &mut pixels, count, image_type, data)?;
    Ok(ColorImage::new(image_size, pixels))
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
    let source =
        unsafe { std::slice::from_raw_parts(data_ptr, pixel_count * image_type.bytes_per_pixel()) };
    let target = unsafe {
        std::slice::from_raw_parts_mut(
            image_ptr.cast::<std::mem::MaybeUninit<u8>>(),
            pixel_count * 4,
        )
    };
    to_rgba(image_type, source, target);
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
            Ok(Some(ChannelMessage::Ack(id) | ChannelMessage::AckKey(id, _))) => {
                assert_eq!(id, expected_id)
            }
            _ => panic!("expected an ACK for {expected_id}"),
        }
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn oversized_wire_images_preserve_textures_buffers_and_ack_behavior() {
        for received in [false, true] {
            let context = egui::Context::default();
            let (sender, mut receiver) = MessageSender::new();
            let image = Image::<()>::new("image".into(), 1, sender, context.clone());
            let (sender, mut multi_receiver) = MessageSender::new();
            let images = ImageMulti::<()>::new("images".into(), 2, sender, context.clone());
            if received {
                image
                    .fill_image([1, 1], [0; 4], HeaderUpdate::Keep, &[])
                    .unwrap();
                assert_single_ack(&mut receiver, 1);
                images
                    .fill_image(7, [1, 1], [0; 4], HeaderUpdate::Keep, &[])
                    .unwrap();
                assert_single_ack(&mut multi_receiver, 2);
            }
            let before = image.get();
            let multi_before = images.get(7);
            crate::test_support::texture_updates(&context);

            for size in [[65_537, 1], [1, 65_537]] {
                for operation in 0..4 {
                    let result = match operation {
                        0 => image.set_image(
                            ImageSetHeader::All(size, HeaderUpdate::Keep),
                            ImageType::Gray,
                            &[],
                        ),
                        1 => image.set_image(ImageSetHeader::Start(size, 1), ImageType::Gray, &[9]),
                        2 => image.fill_image(size, [9; 4], HeaderUpdate::Keep, &[]),
                        _ => image.update_image(
                            ImageUpdateHeader::All([0, 0, size[0], size[1]], HeaderUpdate::Keep),
                            ImageType::Gray,
                            &[],
                        ),
                    };
                    assert!(result.unwrap_err().contains("65536"));
                    let result = match operation {
                        0 => images.set_image(
                            7,
                            ImageSetHeader::All(size, HeaderUpdate::Keep),
                            ImageType::Gray,
                            &[],
                        ),
                        1 => images.set_image(
                            7,
                            ImageSetHeader::Start(size, 1),
                            ImageType::Gray,
                            &[9],
                        ),
                        2 => images.fill_image(7, size, [9; 4], HeaderUpdate::Keep, &[]),
                        _ => images.update_image(
                            7,
                            ImageUpdateHeader::All([0, 0, size[0], size[1]], HeaderUpdate::Keep),
                            ImageType::Gray,
                            &[],
                        ),
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
                assert!(
                    image
                        .update_image(
                            ImageUpdateHeader::All(rect, HeaderUpdate::Keep),
                            ImageType::Gray,
                            &[9]
                        )
                        .is_err()
                );
                assert!(
                    images
                        .update_image(
                            7,
                            ImageUpdateHeader::All(rect, HeaderUpdate::Keep),
                            ImageType::Gray,
                            &[9]
                        )
                        .is_err()
                );
                assert_single_ack(&mut receiver, 1);
                assert_single_ack(&mut multi_receiver, 2);
            }
            assert_eq!(image.get(), before);
            assert_eq!(images.get(7), multi_before);
            assert!(crate::test_support::texture_updates(&context).is_empty());
        }
    }

    /// The options each upload of an image texture used since the last
    /// call; egui's own font texture is left out.
    fn uploaded_options(context: &egui::Context) -> Vec<egui::TextureOptions> {
        let mut uploads: Vec<_> = crate::test_support::texture_updates(context)
            .into_iter()
            .filter(|(id, _)| *id != egui::TextureId::default())
            .flat_map(|(_, deltas)| deltas)
            .map(|delta| delta.options)
            .collect();
        uploads.dedup();
        uploads
    }

    /// Options start as nearest, apply from the next upload of an existing
    /// texture, a complete image or an update, and to textures created
    /// afterwards, and are shared by clones.
    #[test]
    fn texture_options_apply_from_the_next_upload() {
        let linear = egui::TextureOptions::LINEAR;
        let context = egui::Context::default();
        let (sender, _receiver) = MessageSender::new();
        let image = Image::<()>::new("image".into(), 1, sender, context.clone());
        let (sender, _receiver) = MessageSender::new();
        let images = ImageMulti::<()>::new("images".into(), 2, sender, context.clone());
        let gray = |pixels: &[u8]| pixels.to_vec();
        let set = ImageSetHeader::All([1, 1], HeaderUpdate::Keep);
        let update = ImageUpdateHeader::All([0, 0, 1, 1], HeaderUpdate::Keep);

        assert_eq!(image.texture_options(), egui::TextureOptions::NEAREST);
        image.set_image(set, ImageType::Gray, &gray(&[1])).unwrap();
        images
            .set_image(7, set, ImageType::Gray, &gray(&[1]))
            .unwrap();
        assert_eq!(uploaded_options(&context), [egui::TextureOptions::NEAREST]);

        // Changing them leaves the textures alone until their next upload.
        image.set_texture_options(linear);
        images.set_texture_options(linear);
        assert!(uploaded_options(&context).is_empty());
        assert_eq!(image.clone().texture_options(), linear);

        image
            .update_image(update, ImageType::Gray, &gray(&[2]))
            .unwrap();
        assert_eq!(uploaded_options(&context), [linear]);
        images
            .update_image(7, update, ImageType::Gray, &gray(&[2]))
            .unwrap();
        assert_eq!(uploaded_options(&context), [linear]);
        images
            .set_image(8, set, ImageType::Gray, &gray(&[3]))
            .unwrap();
        image.set_image(set, ImageType::Gray, &gray(&[3])).unwrap();
        assert_eq!(uploaded_options(&context), [linear]);

        // A placeholder made after the change uses it too.
        let (sender, _receiver) = MessageSender::new();
        let placeholder = Image::<()>::new("placeholder".into(), 3, sender, context.clone());
        placeholder.set_texture_options(linear);
        placeholder
            .initialize(ColorImage::filled([1, 1], egui::Color32::BLACK))
            .unwrap();
        assert_eq!(uploaded_options(&context), [linear]);
    }

    #[test]
    fn received_images_accept_the_inclusive_limit() {
        for size in [[65_536, 1], [1, 65_536]] {
            // The first image creates the texture before any frame has
            // reported the renderer's limit; egui assumes 2048 until then.
            let context = egui::Context::default();
            let (sender, mut receiver) = MessageSender::new();
            let image = Image::<()>::new("image".into(), 1, sender, context.clone());
            let (sender, mut multi_receiver) = MessageSender::new();
            let images = ImageMulti::<()>::new("images".into(), 2, sender, context.clone());

            let wire_size = [size[0] as u32, size[1] as u32];
            let data = vec![7; 65_536];
            image
                .set_image(
                    ImageSetHeader::All(wire_size, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &data,
                )
                .unwrap();
            images
                .set_image(
                    7,
                    ImageSetHeader::All(wire_size, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &data,
                )
                .unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);

            image
                .set_image(ImageSetHeader::Start(wire_size, 1), ImageType::Gray, &[8])
                .unwrap();
            images
                .set_image(
                    7,
                    ImageSetHeader::Start(wire_size, 1),
                    ImageType::Gray,
                    &[8],
                )
                .unwrap();
            image
                .set_image(
                    ImageSetHeader::End(65_535, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &data[..65_535],
                )
                .unwrap();
            images
                .set_image(
                    7,
                    ImageSetHeader::End(65_535, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &data[..65_535],
                )
                .unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);

            image
                .fill_image(wire_size, [9; 4], HeaderUpdate::Keep, &[])
                .unwrap();
            images
                .fill_image(7, wire_size, [9; 4], HeaderUpdate::Keep, &[])
                .unwrap();
            assert_single_ack(&mut receiver, 1);
            assert_single_ack(&mut multi_receiver, 2);
            let rect = [wire_size[0] - 1, wire_size[1] - 1, 1, 1];
            image
                .update_image(
                    ImageUpdateHeader::All(rect, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[10],
                )
                .unwrap();
            images
                .update_image(
                    7,
                    ImageUpdateHeader::All(rect, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[10],
                )
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
                (ImageSetHeader::Start([3, 1], 1), &[1][..]),
                (ImageSetHeader::Batch(2), &[2, 3][..]),
                (ImageSetHeader::End(0, HeaderUpdate::Keep), &[][..]),
            ]
        };
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::<()>::new("image".into(), 1, sender, context.clone());
        let (sender, mut multi_receiver) = MessageSender::new();
        let images = ImageMulti::<()>::new("images".into(), 2, sender, context);

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

    /// A batch from a closed connection can never complete, so the client
    /// drops it on disconnect and a later End fails with one acknowledgement.
    #[test]
    fn a_disconnect_drops_partial_image_batches() {
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::<()>::new("image".into(), 1, sender, context.clone());
        let (sender, mut multi_receiver) = MessageSender::new();
        let images = ImageMulti::<()>::new("images".into(), 2, sender, context);

        image
            .set_image(ImageSetHeader::Start([2, 1], 1), ImageType::Gray, &[1])
            .unwrap();
        images
            .set_image(7, ImageSetHeader::Start([2, 1], 1), ImageType::Gray, &[1])
            .unwrap();
        image.disconnected();
        images.disconnected();

        assert!(
            image
                .set_image(
                    ImageSetHeader::End(1, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2]
                )
                .is_err()
        );
        assert!(
            images
                .set_image(
                    7,
                    ImageSetHeader::End(1, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2]
                )
                .is_err()
        );
        assert_single_ack(&mut receiver, 1);
        assert_single_ack(&mut multi_receiver, 2);
        assert_eq!(image.get_size(), None);
        assert_eq!(images.get_size(7), None);
    }

    /// A chunked update is applied once, at its End, with a single
    /// acknowledgement; set and update chunks never complete each other.
    #[test]
    fn chunked_updates_are_applied_once_at_their_end() {
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::<()>::new("image".into(), 1, sender, context.clone());
        let (sender, mut multi_receiver) = MessageSender::new();
        let images = ImageMulti::<()>::new("images".into(), 2, sender, context.clone());
        image
            .fill_image([4, 3], [0; 4], HeaderUpdate::Keep, &[])
            .unwrap();
        images
            .fill_image(7, [4, 3], [0; 4], HeaderUpdate::Keep, &[])
            .unwrap();
        assert_single_ack(&mut receiver, 1);
        assert_single_ack(&mut multi_receiver, 2);
        let (id, _) = image.get().unwrap();
        let (multi_id, _) = images.get(7).unwrap();
        crate::test_support::texture_updates(&context);

        // A 2x2 rectangle at x 1, y 1, in three chunks.
        let rect = [1, 1, 2, 2];
        let chunks = || {
            [
                (ImageUpdateHeader::Start(rect, 1), &[1][..]),
                (ImageUpdateHeader::Batch(2), &[2, 3][..]),
                (ImageUpdateHeader::End(1, HeaderUpdate::Keep), &[4][..]),
            ]
        };
        for (n, (header, data)) in chunks().into_iter().enumerate() {
            let last = n == 2;
            image.update_image(header, ImageType::Gray, data).unwrap();
            if !last {
                assert!(receiver.try_recv().is_err());
                assert!(crate::test_support::texture_updates(&context).is_empty());
            }
        }
        for (n, (header, data)) in chunks().into_iter().enumerate() {
            let last = n == 2;
            images
                .update_image(7, header, ImageType::Gray, data)
                .unwrap();
            if !last {
                assert!(multi_receiver.try_recv().is_err());
            }
        }
        assert_single_ack(&mut receiver, 1);
        assert_single_ack(&mut multi_receiver, 2);
        let delta = crate::test_support::texture_updates(&context);
        for texture in [id, multi_id] {
            let changes = &delta[&texture];
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].pos, Some([1, 1]));
            assert_eq!(changes[0].image.size(), [2, 2]);
        }
        assert!(image.buffer.lock().is_none());
        assert!(images.buffers.lock().is_empty());

        // A set's chunks cannot complete an update, nor the reverse; only the
        // failing End is acknowledged.
        image
            .update_image(ImageUpdateHeader::Start(rect, 1), ImageType::Gray, &[1])
            .unwrap();
        assert!(
            image
                .set_image(
                    ImageSetHeader::End(3, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2, 3, 4]
                )
                .is_err()
        );
        assert_single_ack(&mut receiver, 1);
        image
            .set_image(ImageSetHeader::Start([4, 3], 1), ImageType::Gray, &[1])
            .unwrap();
        assert!(
            image
                .update_image(ImageUpdateHeader::Batch(1), ImageType::Gray, &[2])
                .is_err()
        );
        assert!(
            image
                .update_image(
                    ImageUpdateHeader::End(3, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2, 3, 4]
                )
                .is_err()
        );
        assert_single_ack(&mut receiver, 1);
        images
            .update_image(7, ImageUpdateHeader::Start(rect, 1), ImageType::Gray, &[1])
            .unwrap();
        assert!(
            images
                .set_image(7, ImageSetHeader::Batch(1), ImageType::Gray, &[2])
                .is_err()
        );
        assert!(multi_receiver.try_recv().is_err());

        // A disconnect drops a partial update, so its End fails.
        image
            .update_image(ImageUpdateHeader::Start(rect, 1), ImageType::Gray, &[1])
            .unwrap();
        image.disconnected();
        images.disconnected();
        assert!(
            image
                .update_image(
                    ImageUpdateHeader::End(3, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2, 3, 4]
                )
                .is_err()
        );
        assert!(
            images
                .update_image(
                    7,
                    ImageUpdateHeader::End(3, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2, 3, 4]
                )
                .is_err()
        );
        assert_single_ack(&mut receiver, 1);
        assert_single_ack(&mut multi_receiver, 2);

        // An update rectangle outside the texture is rejected at its Start.
        assert!(
            image
                .update_image(
                    ImageUpdateHeader::Start([3, 1, 2, 2], 1),
                    ImageType::Gray,
                    &[1]
                )
                .is_err()
        );
        assert!(image.buffer.lock().is_none());
        assert!(receiver.try_recv().is_err());
        assert!(crate::test_support::texture_updates(&context).is_empty());
    }

    #[test]
    fn invalid_fill_messages_still_ack() {
        let id = 51;
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::<()>::new("image".to_string(), id, sender, egui::Context::default());

        assert!(
            image
                .fill_image([1, 1], [0, 0, 0, 0], HeaderUpdate::Keep, &[1])
                .is_err()
        );
        assert_single_ack(&mut receiver, id);

        assert!(
            image
                .fill_image([0, 1], [0, 0, 0, 0], HeaderUpdate::Keep, &[])
                .is_err()
        );
        assert_single_ack(&mut receiver, id);
    }

    #[test]
    fn valid_fill_keeps_its_existing_single_ack() {
        let id = 52;
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::<()>::new("image".to_string(), id, sender, egui::Context::default());

        image
            .fill_image([1, 1], [0, 0, 0, 0], HeaderUpdate::Keep, &[])
            .unwrap();
        assert_single_ack(&mut receiver, id);
    }

    /// The first complete image creates the texture, whichever message carries
    /// it, and later images reuse that texture.
    #[test]
    fn the_first_complete_image_creates_the_texture() {
        let deliveries: [fn(&Image); 3] = [
            |image| {
                image
                    .set_image(
                        ImageSetHeader::All([2, 1], HeaderUpdate::Keep),
                        ImageType::Gray,
                        &[1, 2],
                    )
                    .unwrap()
            },
            |image| {
                image
                    .set_image(ImageSetHeader::Start([2, 1], 1), ImageType::Gray, &[1])
                    .unwrap();
                image
                    .set_image(
                        ImageSetHeader::End(1, HeaderUpdate::Keep),
                        ImageType::Gray,
                        &[2],
                    )
                    .unwrap();
            },
            |image| {
                image
                    .fill_image([2, 1], [1, 2, 3, 255], HeaderUpdate::Keep, &[])
                    .unwrap()
            },
        ];
        for deliver in deliveries {
            let context = egui::Context::default();
            let (sender, mut receiver) = MessageSender::new();
            let image = Image::<()>::new("image".into(), 1, sender, context.clone());
            crate::test_support::texture_updates(&context);
            assert!(image.get().is_none());

            let error = image
                .update_image(
                    ImageUpdateHeader::All([0, 0, 1, 1], HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[9],
                )
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

            image
                .fill_image([3, 2], [0; 4], HeaderUpdate::Keep, &[])
                .unwrap();
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
                .set_image(
                    ImageSetHeader::All([3, 1], HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[1, 2, 3],
                )
                .unwrap()
        };

        // An empty or oversized placeholder creates nothing.
        let context = egui::Context::default();
        let (sender, mut receiver) = MessageSender::new();
        let image = Image::<()>::new("image".into(), 1, sender, context.clone());
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
        let image = Image::<()>::new("image".into(), 1, sender, context.clone());
        server_image(&image);
        assert_single_ack(&mut receiver, 1);
        let received = image.get();
        crate::test_support::texture_updates(&context);
        image.initialize(placeholder([2, 2])).unwrap();
        assert_eq!(image.get(), received);
        assert!(crate::test_support::texture_updates(&context).is_empty());
    }

    #[test]
    fn image_multi_acknowledges_each_index_on_its_own() {
        let (sender, mut receiver) = MessageSender::new();
        let images =
            ImageMulti::<()>::new("images".to_string(), 54, sender, egui::Context::default());
        images
            .set_image(
                7,
                ImageSetHeader::All([1, 1], HeaderUpdate::Keep),
                ImageType::Gray,
                &[10],
            )
            .unwrap();
        images
            .fill_image(2, [1, 1], [1, 2, 3, 4], HeaderUpdate::Keep, &[])
            .unwrap();
        for key in [7, 2] {
            assert!(matches!(
                receiver.try_recv(),
                Ok(Some(ChannelMessage::AckKey(54, acked))) if acked == key
            ));
        }
    }

    #[test]
    fn image_multi_creates_replaces_and_clears_sparse_textures() {
        let id = 53;
        let (sender, mut receiver) = MessageSender::new();
        let images =
            ImageMulti::<()>::new("images".to_string(), id, sender, egui::Context::default());

        // The first image for a key creates its texture without any setup.
        images
            .set_image(
                7,
                ImageSetHeader::All([2, 1], HeaderUpdate::Keep),
                ImageType::Gray,
                &[10, 20],
            )
            .unwrap();
        assert_single_ack(&mut receiver, id);
        images
            .fill_image(2, [1, 3], [1, 2, 3, 4], HeaderUpdate::Keep, &[])
            .unwrap();
        assert_single_ack(&mut receiver, id);

        assert_eq!(images.len(), 2);
        assert_eq!(images.indices(), vec![2, 7]);
        assert!(images.contains(7));
        assert_eq!(images.get_size(7), Some([2, 1]));
        assert_eq!(images.get_size(2), Some([1, 3]));

        images
            .fill_image(7, [3, 2], [4, 3, 2, 1], HeaderUpdate::Keep, &[])
            .unwrap();
        assert_single_ack(&mut receiver, id);
        assert_eq!(images.len(), 2);
        assert_eq!(images.get_size(7), Some([3, 2]));

        assert!(
            images
                .update_image(
                    99,
                    ImageUpdateHeader::All([0, 0, 1, 1], HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[5],
                )
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
        let images = ImageMulti::<()>::new("images".to_string(), id, sender, context.clone());
        crate::test_support::texture_updates(&context);

        for index in [2, 7] {
            images
                .set_image(
                    index,
                    ImageSetHeader::Start([2, 1], 1),
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
                ImageSetHeader::End(1, HeaderUpdate::Keep),
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
                ImageSetHeader::End(1, HeaderUpdate::Keep),
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
            .set_image(11, ImageSetHeader::Start([2, 1], 1), ImageType::Gray, &[1])
            .unwrap();
        images.remove(11);
        assert!(
            images
                .set_image(
                    11,
                    ImageSetHeader::End(1, HeaderUpdate::Keep),
                    ImageType::Gray,
                    &[2],
                )
                .is_err()
        );
        assert_single_ack(&mut receiver, id);

        images
            .set_image(12, ImageSetHeader::Start([2, 1], 1), ImageType::Gray, &[1])
            .unwrap();
        images.reset();
        assert!(images.buffers.lock().is_empty());
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;
    use crate::client::messages::MessageSender;
    use crate::typed::Typed;

    type Meta = (u32, String);

    fn meta(n: u32) -> Meta {
        (n, format!("m{n}"))
    }

    fn type_id() -> u32 {
        <Meta as Typed>::get_type().get_hash()
    }

    /// The header update and payload of `header` followed by `pixels`.
    fn payload(header: &Meta, pixels: &[u8]) -> (HeaderUpdate, Vec<u8>) {
        let mut bytes = postcard::to_stdvec(header).unwrap();
        let update = HeaderUpdate::Set(HeaderBytes {
            type_id: type_id(),
            size: bytes.len() as u32,
        });
        bytes.extend_from_slice(pixels);
        (update, bytes)
    }

    fn image() -> Image<Meta> {
        let (sender, _receiver) = MessageSender::new();
        Image::new("image".into(), 1, sender, egui::Context::default())
    }

    fn images() -> ImageMulti<Meta> {
        let (sender, _receiver) = MessageSender::new();
        ImageMulti::new("images".into(), 2, sender, egui::Context::default())
    }

    fn set(image: &Image<Meta>, update: HeaderUpdate, payload: &[u8]) -> Result<(), String> {
        let header = ImageSetHeader::All([2, 1], update);
        image.apply(ImageHeader::Set(header, ImageType::Gray), payload)
    }

    #[test]
    fn a_header_is_published_with_its_image_and_kept_without_one() {
        let image = image();
        assert_eq!(image.header(), None);
        assert!(image.get_with_header().is_none());

        let (update, bytes) = payload(&meta(1), &[1, 2]);
        set(&image, update, &bytes).unwrap();
        let (id, size, header) = image.get_with_header().unwrap();
        assert_eq!((size, header), ([2, 1], meta(1)));

        // An update without a header keeps it; one with a header replaces it
        // together with the rectangle.
        let rect = |update| ImageUpdateHeader::All([0, 0, 1, 1], update);
        let update = |header, pixels: &[u8]| {
            image.apply(ImageHeader::Update(header, ImageType::Gray), pixels)
        };
        update(rect(HeaderUpdate::Keep), &[3]).unwrap();
        assert_eq!(image.header(), Some(meta(1)));
        let (header_update, bytes) = payload(&meta(2), &[4]);
        update(rect(header_update), &bytes).unwrap();
        assert_eq!(image.get_with_header(), Some((id, [2, 1], meta(2))));
        image.read_header(|header| assert_eq!(header, Some(&meta(2))));

        // A fill and a header alone replace it too.
        let (update, bytes) = payload(&meta(3), &[]);
        image
            .apply(ImageHeader::Fill([1, 1], [0; 4], update), &bytes)
            .unwrap();
        assert_eq!(image.get_with_header(), Some((id, [1, 1], meta(3))));
        let HeaderUpdate::Set(header_bytes) = payload(&meta(4), &[]).0 else {
            unreachable!()
        };
        let bytes = postcard::to_stdvec(&meta(4)).unwrap();
        image
            .apply(ImageHeader::Header(header_bytes), &bytes)
            .unwrap();
        assert_eq!(image.get_with_header(), Some((id, [1, 1], meta(4))));
    }

    #[test]
    fn a_split_image_gets_the_header_of_its_last_chunk() {
        let image = image();
        let start = ImageSetHeader::Start([2, 1], 1);
        image
            .apply(ImageHeader::Set(start, ImageType::Gray), &[1])
            .unwrap();
        assert!(image.get().is_none());
        let (update, bytes) = payload(&meta(5), &[2]);
        let end = ImageSetHeader::End(1, update);
        image
            .apply(ImageHeader::Set(end, ImageType::Gray), &bytes)
            .unwrap();
        assert_eq!(image.header(), Some(meta(5)));
    }

    #[test]
    fn a_bad_or_missing_header_changes_nothing() {
        let image = image();
        // An image of a state with a header type needs one.
        assert!(set(&image, HeaderUpdate::Keep, &[1, 2]).is_err());
        assert!(image.get().is_none());
        // A header alone needs an image.
        let (HeaderUpdate::Set(header_bytes), bytes) = payload(&meta(1), &[]) else {
            unreachable!()
        };
        assert!(
            image
                .apply(ImageHeader::Header(header_bytes), &bytes)
                .is_err()
        );

        let (update, bytes) = payload(&meta(1), &[1, 2]);
        set(&image, update, &bytes).unwrap();
        // A header of another type, or one that does not decode, leaves the
        // image and its header as they were.
        let wrong = HeaderUpdate::Set(HeaderBytes {
            type_id: type_id() ^ 1,
            size: bytes.len() as u32 - 2,
        });
        assert!(set(&image, wrong, &bytes).is_err());
        let broken = HeaderUpdate::Set(HeaderBytes {
            type_id: type_id(),
            size: 1,
        });
        assert!(set(&image, broken, &[0xff, 5, 6]).is_err());
        assert_eq!(
            image.get_with_header().map(|(_, size, h)| (size, h)),
            Some(([2, 1], meta(1)))
        );
    }

    #[test]
    fn a_placeholder_has_a_header_only_when_given_one() {
        let placeholder = || ColorImage::filled([1, 1], egui::Color32::BLACK);
        let image = image();
        image.initialize(placeholder()).unwrap();
        assert!(image.get().is_some());
        assert_eq!(image.header(), None);
        assert!(image.get_with_header().is_none());
        // The first server image brings its header.
        let (update, bytes) = payload(&meta(1), &[1, 2]);
        set(&image, update, &bytes).unwrap();
        assert_eq!(image.header(), Some(meta(1)));

        let image = self::image();
        image
            .initialize_with_header(placeholder(), meta(9))
            .unwrap();
        assert_eq!(image.header(), Some(meta(9)));
        // Once the texture exists, neither replaces it or its header.
        image
            .initialize_with_header(placeholder(), meta(8))
            .unwrap();
        assert_eq!(image.header(), Some(meta(9)));
        let (update, bytes) = payload(&meta(2), &[1, 2]);
        set(&image, update, &bytes).unwrap();
        assert_eq!(image.header(), Some(meta(2)));

        // Without a header type, the header is `()` wherever a texture is.
        let (sender, _receiver) = MessageSender::new();
        let plain = Image::<()>::new("plain".into(), 3, sender, egui::Context::default());
        assert_eq!(plain.header(), None);
        plain.initialize(placeholder()).unwrap();
        assert_eq!(plain.header(), Some(()));
    }

    #[test]
    fn every_key_has_its_own_header() {
        let images = images();
        let set = |index, update, payload: &[u8]| {
            let header = ImageSetHeader::All([1, 1], update);
            images.apply(index, ImageHeader::Set(header, ImageType::Gray), payload)
        };
        // A new key needs a header.
        assert!(set(4, HeaderUpdate::Keep, &[1]).is_err());
        assert!(!images.contains(4));
        let (update, bytes) = payload(&meta(4), &[1]);
        set(4, update, &bytes).unwrap();
        let (update, bytes) = payload(&meta(5), &[1]);
        set(5, update, &bytes).unwrap();
        // A change of a key without a header keeps that key's header.
        set(4, HeaderUpdate::Keep, &[2]).unwrap();
        assert_eq!(images.header(4), Some(meta(4)));
        assert_eq!(images.get_with_header(5).map(|(_, _, h)| h), Some(meta(5)));
        images.read_header(6, |header| assert_eq!(header, None));

        let (HeaderUpdate::Set(header_bytes), bytes) = payload(&meta(7), &[]) else {
            unreachable!()
        };
        images
            .apply(5, ImageHeader::Header(header_bytes), &bytes)
            .unwrap();
        assert_eq!(images.header(5), Some(meta(7)));
        assert!(
            images
                .apply(6, ImageHeader::Header(header_bytes), &bytes)
                .is_err()
        );

        images.remove(4);
        assert_eq!(images.header(4), None);
        images.reset();
        assert_eq!(images.header(5), None);
    }
}
