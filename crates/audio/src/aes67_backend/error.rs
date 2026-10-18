#[derive(Debug, thiserror::Error)]
pub enum Aes67Error {
    #[error("SAP: {0}")]
    Sap(String),

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("Read error: {0}")]
    BufferReaderError(String),
}
