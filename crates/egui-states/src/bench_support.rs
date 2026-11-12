//! Entry points for the `egui_states_bench` benchmarks.
//!
//! Not part of the public API: these wrappers expose internal hot paths so they
//! can be measured in isolation, and they may change with any release.

use bytes::Bytes;

use crate::client::messages::MessageSender;
use crate::client::states::data::{Data, UpdateData};
use crate::protocol::data::{DataHeader, DataType, HeaderUpdate::Keep, TransportType};
use crate::protocol::image::ImageType;
use crate::server_core::states::data::{WireHeader, pack_data};
use crate::server_core::states::image::single::{ImageData, pack_set_data};

/// A pixel layout accepted by the server and sent to the client.
#[derive(Clone, Copy, Debug)]
pub enum PixelFormat {
    /// One byte per pixel.
    Gray,
    /// Two bytes per pixel.
    GrayAlpha,
    /// Three bytes per pixel.
    Color,
    /// Four bytes per pixel.
    ColorAlpha,
}

impl PixelFormat {
    fn image_type(self) -> ImageType {
        match self {
            Self::Gray => ImageType::Gray,
            Self::GrayAlpha => ImageType::GrayAlpha,
            Self::Color => ImageType::Color,
            Self::ColorAlpha => ImageType::ColorAlpha,
        }
    }

    /// Bytes per source pixel.
    pub fn bytes_per_pixel(self) -> usize {
        self.image_type().bytes_per_pixel()
    }
}

/// Packs a `u8` Data set of `data` and returns the total message size.
pub fn pack_data_set(data: &[u8]) -> usize {
    let count = data.len() as u64;
    pack_data(
        1,
        data,
        TransportType::Set(count),
        count,
        DataType::U8,
        None,
        false,
        WireHeader::KEEP,
    )
    .expect("pack data")
    .iter()
    .map(|(message, _)| message.len())
    .sum()
}

/// Packs an image set of `pixels` in `format` and returns the total message size.
pub fn pack_image_set(pixels: &[u8], size: [usize; 2], format: PixelFormat) -> usize {
    let image_type = format.image_type();
    assert_eq!(
        pixels.len(),
        size[0] * size[1] * image_type.bytes_per_pixel()
    );
    let image = ImageData::packed(pixels, size, image_type);
    pack_set_data(1, &image, false)
        .expect("pack image")
        .iter()
        .map(|(message, _)| message.len())
        .sum()
}

/// Converts `pixels` to the server's RGBA storage, as an image set of an
/// unchanged size does.
pub fn server_to_rgba(pixels: &[u8], format: PixelFormat, rgba: &mut Vec<u8>) {
    let image_type = format.image_type();
    let count = pixels.len() / image_type.bytes_per_pixel();
    assert_eq!(rgba.len(), count * 4);
    let image = ImageData::packed(pixels, [1, count], image_type);
    crate::server_core::states::image::common::store_rgba(rgba, &image, false);
}

/// Builds the client texture image for `pixels`, as a complete image set does.
pub fn client_color_image(
    pixels: &[u8],
    size: [usize; 2],
    format: PixelFormat,
) -> egui::ColorImage {
    // The client works in egui's `[width, height]` order.
    crate::client::states::image::create_color_image(
        &"bench",
        [size[1], size[0]],
        format.image_type(),
        pixels,
    )
    .expect("client image")
}

/// A client `Data<u8>` handle fed directly with transfer messages.
pub struct ClientData {
    data: Data<u8>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<Option<crate::client::messages::ChannelMessage>>,
}

impl ClientData {
    /// Creates a disconnected client handle.
    pub fn new() -> Self {
        let (sender, receiver) = MessageSender::new();
        Self {
            data: Data::new("bench".to_string(), 1, sender),
            receiver,
        }
    }

    /// Applies a chunked set of `chunks`, as the client receive task does, and
    /// returns the length of the published buffer.
    pub fn assemble(&mut self, chunks: &[Bytes]) -> usize {
        let total: usize = chunks.iter().map(Bytes::len).sum();
        let count = total as u64;
        match chunks {
            [] => {}
            [single] => self
                .data
                .update_data((
                    DataHeader::All(DataType::U8, TransportType::Set(count), false, 0, Keep),
                    single.clone(),
                ))
                .expect("set"),
            [first, middle @ .., last] => {
                self.data
                    .update_data((DataHeader::StartBatch(count, 0), first.clone()))
                    .expect("start");
                for chunk in middle {
                    self.data
                        .update_data((DataHeader::Batch(0), chunk.clone()))
                        .expect("batch");
                }
                self.data
                    .update_data((
                        DataHeader::End(DataType::U8, TransportType::Set(count), false, 0, Keep),
                        last.clone(),
                    ))
                    .expect("end");
            }
        }
        while self.receiver.try_recv().is_ok() {}
        self.data.read(|data| data.map_or(0, <[u8]>::len))
    }
}

impl Default for ClientData {
    fn default() -> Self {
        Self::new()
    }
}
