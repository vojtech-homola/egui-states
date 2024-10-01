use crate::transport::SIZE_START;

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

#[derive(PartialEq, Debug)]
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
    pub fn write_message(self, head: &mut [u8]) -> Vec<u8> {
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
                head[6..8].copy_from_slice(&(rec[0] as u16).to_le_bytes());
                head[8..10].copy_from_slice(&(rec[1] as u16).to_le_bytes());
                head[10..12].copy_from_slice(&(rec[2] as u16).to_le_bytes());
                head[12..14].copy_from_slice(&(rec[3] as u16).to_le_bytes());
            }
            None => head[6] = 0,
        }
        head[SIZE_START..].copy_from_slice(&(self.data.len() as u64).to_le_bytes());

        self.data
    }

    pub fn read_message(head: &mut [u8], data: Option<Vec<u8>>) -> Result<Self, String> {
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
                u16::from_le_bytes(head[6..8].try_into().unwrap()) as usize,
                u16::from_le_bytes(head[8..10].try_into().unwrap()) as usize,
                u16::from_le_bytes(head[10..12].try_into().unwrap()) as usize,
                u16::from_le_bytes(head[12..14].try_into().unwrap()) as usize,
            ];
            let data_size = rectangle[2] * rectangle[3];
            (Some(rectangle), data_size)
        } else {
            (None, y * x)
        };
        let size = u64::from_le_bytes(head[21..29].try_into().unwrap()) as usize;

        let is_right = match image_type {
            ImageType::Color => size == data_size * 3,
            ImageType::ColorAlpha => size == data_size * 4,
            ImageType::Gray => size == data_size,
            ImageType::GrayAlpha => size == data_size * 2,
        };

        if !is_right {
            return Err(format!("Wrong size of the image data: {}", size));
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

// histogram ------------------------------------------------------------------
/*
histogram head:
| 4B - u32 histogram size | ... | 8B - u64 data size |
*/
pub struct HistogramMessage(pub Option<Vec<f32>>);

impl HistogramMessage {
    pub fn write_message(self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self.0 {
            Some(hist) => {
                let size = hist.len();
                let data_size = size * std::mem::size_of::<f32>();
                let mut data = vec![0u8; data_size];

                // TODO: possibly do it witoout copying
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        hist.as_ptr(),
                        data.as_mut_ptr() as *mut f32,
                        size,
                    );
                }

                head[0..4].copy_from_slice(&(size as u32).to_le_bytes());
                head[SIZE_START..].copy_from_slice(&(data_size as u64).to_le_bytes());
                Some(data)
            }
            None => {
                head[0..4].copy_from_slice(&0u32.to_le_bytes());
                head[SIZE_START..].copy_from_slice(&0u64.to_le_bytes());
                None
            }
        }
    }

    pub fn read_message(head: &mut [u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let size = u32::from_le_bytes(head[0..4].try_into().unwrap()) as usize;
        let data_size = u64::from_le_bytes(head[SIZE_START..].try_into().unwrap()) as usize;

        let data = match data {
            Some(data) => {
                if size * std::mem::size_of::<f32>() != data.len() || data_size != data.len() {
                    return Err("Histogram data parsing failed.".to_string());
                }

                let mut hist = vec![0.0f32; size];
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr() as *mut f32,
                        hist.as_mut_ptr(),
                        size,
                    );
                }

                Some(hist)
            }

            None => {
                if size != 0 {
                    return Err("Histogram data parsing failed.".to_string());
                }
                None
            }
        };

        Ok(HistogramMessage(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HEAD_SIZE;

    #[test]
    fn test_image_message() {
        let mut head = [0u8; HEAD_SIZE];
        let data = vec![0u8; 5 * 5 * 3];

        let message = ImageMessage {
            image_size: [10, 10],
            rect: Some([0, 0, 5, 5]),
            data,
            image_type: ImageType::Color,
        };

        let data = message.write_message(&mut head[6..]);
        let message = ImageMessage::read_message(&mut head[6..], Some(data)).unwrap();

        assert_eq!(message.image_size, [10, 10]);
        assert_eq!(message.rect, Some([0, 0, 5, 5]));
        assert_eq!(message.data.len(), 5 * 5 * 3);
        assert_eq!(message.image_type, ImageType::Color);
    }

    #[test]
    fn test_histogram_message() {
        let mut head = [0u8; HEAD_SIZE];
        let original_data = vec![42.0f32; 10];

        let message = HistogramMessage(Some(original_data.clone()));

        let data = message.write_message(&mut head[6..]);
        let message = HistogramMessage::read_message(&mut head[6..], data).unwrap();

        assert_eq!(message.0.unwrap(), original_data);
    }
}
