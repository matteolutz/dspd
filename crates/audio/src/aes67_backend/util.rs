use crate::aes67_backend::error::{Aes67Error, Aes67Result};

pub struct BufferReader<'a>(&'a [u8]);

impl<'a> From<&'a [u8]> for BufferReader<'a> {
    fn from(slice: &'a [u8]) -> Self {
        BufferReader(slice)
    }
}

impl<'a> BufferReader<'a> {
    pub fn read_u8(&mut self) -> Aes67Result<u8> {
        if self.0.is_empty() {
            return Err(Aes67Error::BufferReaderError(
                "Buffer underflow".to_string(),
            ));
        }
        let value = self.0[0];
        self.0 = &self.0[1..];
        Ok(value)
    }

    pub fn read_u16(&mut self) -> Aes67Result<u16> {
        let high = self.read_u8()?;
        let low = self.read_u8()?;
        Ok(u16::from_be_bytes([high, low]))
    }

    pub fn read_u32(&mut self) -> Aes67Result<u32> {
        let b1 = self.read_u8()?;
        let b2 = self.read_u8()?;
        let b3 = self.read_u8()?;
        let b4 = self.read_u8()?;
        Ok(u32::from_be_bytes([b1, b2, b3, b4]))
    }

    pub fn read_u64(&mut self) -> Aes67Result<u64> {
        let b1 = self.read_u8()?;
        let b2 = self.read_u8()?;
        let b3 = self.read_u8()?;
        let b4 = self.read_u8()?;
        let b5 = self.read_u8()?;
        let b6 = self.read_u8()?;
        let b7 = self.read_u8()?;
        let b8 = self.read_u8()?;
        Ok(u64::from_be_bytes([b1, b2, b3, b4, b5, b6, b7, b8]))
    }

    pub fn read_until(&mut self, delimiter: u8) -> Aes67Result<Vec<u8>> {
        let mut result = Vec::new();
        while !self.0.is_empty() {
            let byte = self.read_u8()?;
            if byte == delimiter {
                break;
            }
            result.push(byte);
        }
        Ok(result)
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.0
    }
}

pub trait MediaDescriptionExt {
    fn find_other_attribute(&self, key: &str) -> Option<&str>;
}

impl MediaDescriptionExt for sdp_rs::MediaDescription {
    fn find_other_attribute(&self, key: &str) -> Option<&str> {
        self.attributes.iter().find_map(|attr| {
            if let sdp_rs::lines::Attribute::Other(attr_key, value) = attr {
                if attr_key == key {
                    value.as_deref()
                } else {
                    None
                }
            } else {
                None
            }
        })
    }
}
