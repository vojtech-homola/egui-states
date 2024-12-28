// image ----------------------------------------------------------------------
/*
image head:
| 1B - image_type | 2B - u16 Y | 2B - u16 X | 1B - bool - is rectangle |
| 8B - 4 x 2B - rectangle | ... | 8B - u64 data size |
*/

const IMAGE_COLOR: u8 = 150;
const IMAGE_COLOR_ALPHA: u8 = 151;
const IMAGE_GRAY: u8 = 152;
const IMAGE_GRAY_ALPHA: u8 = 153;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ImageType {
    Color,
    ColorAlpha,
    Gray,
    GrayAlpha,
}

pub struct ImageMessage {
    pub image_size: [usize; 2],   // [y, x]
    pub rect: Option<[usize; 4]>, // [y, x, h, w]
    pub data: Vec<u8>,
    pub image_type: ImageType,
}

impl ImageMessage {
    pub(crate) fn write_message(self, head: &mut [u8]) -> Vec<u8> {
        head[0] = match self.image_type {
            ImageType::Color => IMAGE_COLOR,
            ImageType::ColorAlpha => IMAGE_COLOR_ALPHA,
            ImageType::Gray => IMAGE_GRAY,
            ImageType::GrayAlpha => IMAGE_GRAY_ALPHA,
        };

        head[1..3].copy_from_slice(&(self.image_size[0] as u16).to_le_bytes());
        head[3..5].copy_from_slice(&(self.image_size[1] as u16).to_le_bytes());
        match self.rect {
            Some(rec) => {
                head[6] = 255;
                head[7..9].copy_from_slice(&(rec[0] as u16).to_le_bytes());
                head[9..11].copy_from_slice(&(rec[1] as u16).to_le_bytes());
                head[11..13].copy_from_slice(&(rec[2] as u16).to_le_bytes());
                head[13..15].copy_from_slice(&(rec[3] as u16).to_le_bytes());
            }
            None => head[6] = 0,
        }

        self.data
    }

    pub(crate) fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let data = data.ok_or("No data for the image message".to_string())?;

        let image_type = match head[0] {
            IMAGE_COLOR => ImageType::Color,
            IMAGE_GRAY => ImageType::Gray,
            IMAGE_COLOR_ALPHA => ImageType::ColorAlpha,
            IMAGE_GRAY_ALPHA => ImageType::GrayAlpha,
            _ => return Err("Unknown image type".to_string()),
        };

        let y = u16::from_le_bytes(head[1..3].try_into().unwrap()) as usize;
        let x = u16::from_le_bytes(head[3..5].try_into().unwrap()) as usize;
        let is_rectangle = head[6] != 0;

        let (rectangle, data_size) = if is_rectangle {
            let rectangle = [
                u16::from_le_bytes(head[7..9].try_into().unwrap()) as usize,
                u16::from_le_bytes(head[9..11].try_into().unwrap()) as usize,
                u16::from_le_bytes(head[11..13].try_into().unwrap()) as usize,
                u16::from_le_bytes(head[13..15].try_into().unwrap()) as usize,
            ];

            let data_size = rectangle[2] * rectangle[3];
            (Some(rectangle), data_size)
        } else {
            (None, y * x)
        };
        let size = data.len();

        let is_right = match image_type {
            ImageType::Color => size == data_size * 3,
            ImageType::ColorAlpha => size == data_size * 4,
            ImageType::Gray => size == data_size,
            ImageType::GrayAlpha => size == data_size * 2,
        };

        if !is_right {
            return Err(format!(
                "Wrong size of the image data: {} for data size {}",
                size, data_size
            ));
        }

        let image_data = ImageMessage {
            image_size: [y, x],
            rect: rectangle,
            data,
            image_type,
        };

        Ok(image_data)
    }
}
