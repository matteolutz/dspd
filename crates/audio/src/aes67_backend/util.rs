use crate::aes67_backend::error::Aes67Error;

pub struct BufferReader<'a>(&'a [u8]);

impl<'a> From<&'a [u8]> for BufferReader<'a> {
    fn from(slice: &'a [u8]) -> Self {
        BufferReader(slice)
    }
}

impl<'a> BufferReader<'a> {
    pub fn read_u8(&mut self) -> Result<u8, Aes67Error> {
        if self.0.len() < 1 {
            return Err(Aes67Error::BufferReaderError(
                "Buffer underflow".to_string(),
            ));
        }
        let value = self.0[0];
        self.0 = &self.0[1..];
        Ok(value)
    }

    pub fn read_u16(&mut self) -> Result<u16, Aes67Error> {
        let high = self.read_u8()?;
        let low = self.read_u8()?;
        Ok(u16::from_be_bytes([high, low]))
    }

    pub fn read_u32(&mut self) -> Result<u32, Aes67Error> {
        let b1 = self.read_u8()?;
        let b2 = self.read_u8()?;
        let b3 = self.read_u8()?;
        let b4 = self.read_u8()?;
        Ok(u32::from_be_bytes([b1, b2, b3, b4]))
    }

    pub fn read_u64(&mut self) -> Result<u64, Aes67Error> {
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

    pub fn read_until(&mut self, delimiter: u8) -> Result<Vec<u8>, Aes67Error> {
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
}
