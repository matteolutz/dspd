#[derive(Debug, thiserror::Error)]
pub enum Aes67Error {
    #[error("SAP: {0}")]
    Sap(String),

    #[error("SDP: {0}")]
    Sdp(#[from] sdp_rs::Error),

    #[error("RTP Receiver: {0}")]
    RtpReceiver(String),

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("Read error: {0}")]
    BufferReaderError(String),
}

pub type Aes67Result<T> = Result<T, Aes67Error>;
