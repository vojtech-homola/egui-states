use crate::values::{ReadValue, WriteValue};

// basic types
// -----------------------------------------------------
impl ReadValue for u8 {
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if data.is_some() {
            return Err("u8 value do not accept additional data.".to_string());
        }

        Ok(head[0])
    }
}

impl WriteValue for u8 {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        head[0] = *self;
        None
    }
}

// basic arrays
// -----------------------------------------------------

// bool
impl ReadValue for [bool; 2] {
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if data.is_some() {
            return Err("Bool array do not accept additional data.".to_string());
        }

        Ok([head[0] != 0, head[1] != 0])
    }
}

impl WriteValue for [bool; 2] {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        head[0] = self[0] as u8;
        head[1] = self[1] as u8;
        None
    }
}

impl ReadValue for [bool; 3] {
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if data.is_some() {
            return Err("Bool array do not accept additional data.".to_string());
        }

        Ok([head[0] != 0, head[1] != 0, head[2] != 0])
    }
}

impl WriteValue for [bool; 3] {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        head[0] = self[0] as u8;
        head[1] = self[1] as u8;
        head[2] = self[2] as u8;
        None
    }
}

// 4xf32
impl ReadValue for [f32; 4] {
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if data.is_some() {
            return Err("[f32; 4] array do not accept additional data.".to_string());
        }

        let mut r = [0.0; 4];
        for i in 0..4 {
            r[i] = f32::from_le_bytes(head[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        Ok(r)
    }
}

impl WriteValue for [f32; 4] {
    fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        for i in 0..4 {
            head[i * 4..(i + 1) * 4].copy_from_slice(&self[i].to_le_bytes());
        }
        None
    }
}

// 4xf64
impl ReadValue for [f64; 4] {
    fn read_message(_: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        if let Some(data) = data {
            if data.len() != 32 {
                return Err("[f64; 4] needs 32 bytes.".to_string());
            }

            let mut r = [0.0; 4];
            for i in 0..4 {
                r[i] = f64::from_le_bytes(data[i * 8..(i + 1) * 8].try_into().unwrap());
            }
        }

        return Err("[f64; 4] needs additional data.".to_string());
    }
}

impl WriteValue for [f64; 4] {
    fn write_message(&self, _: &mut [u8]) -> Option<Vec<u8>> {
        let mut data = vec![0u8; 32];
        for i in 0..4 {
            data[i * 8..(i + 1) * 8].copy_from_slice(&self[i].to_le_bytes());
        }
        Some(data)
    }
}
