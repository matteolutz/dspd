use std::net::{IpAddr, Ipv4Addr};

use tokio::sync::mpsc;

use crate::aes67_backend::{
    error::{Aes67Error, Aes67Result},
    util::BufferReader,
};

const MAX_SAP_LENGTH: usize = 4096;

#[derive(Debug, Copy, Clone)]
enum SapAnnouncementAddressType {
    Ipv4,
    Ipv6,
}

impl SapAnnouncementAddressType {
    fn from_u8(value: u8) -> Aes67Result<Self> {
        match value {
            0 => Ok(SapAnnouncementAddressType::Ipv4),
            1 => Ok(SapAnnouncementAddressType::Ipv6),
            _ => Err(Aes67Error::Sap(format!("Invalid address type: {}", value))),
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum SapAnnouncementMessageType {
    Announcement,
    Deletion,
}

impl SapAnnouncementMessageType {
    fn from_u8(value: u8) -> Aes67Result<Self> {
        match value {
            0 => Ok(SapAnnouncementMessageType::Announcement),
            1 => Ok(SapAnnouncementMessageType::Deletion),
            _ => Err(Aes67Error::Sap(format!("Invalid message type: {}", value))),
        }
    }
}

#[derive(Debug, Clone)]
struct SapAnnouncementHeader {
    /// 3-bit version number
    version: u8,

    /// 1-bit address type (0 = IPv4, 1 = IPv6)
    address_type: SapAnnouncementAddressType,

    /// 1-bit message type (0 = announcement, 1 = deletion)
    message_type: SapAnnouncementMessageType,

    /// 1-bit encryption flag (0 = unencrypted, 1 = encrypted)
    is_encrypted: bool,

    /// 1-bit compression flag (0 = uncompressed, 1 = compressed)
    is_compressed: bool,

    /// 8-bit authentication len (0 = no authentication)
    authentication_length: u8,

    /// 16-bit message ID hash
    message_id_hash: u16,

    /// 32-bit source address
    originating_source: IpAddr,

    /// Payload type as string (null-terminated)
    payload_type: String,
}

#[derive(Debug)]
pub struct SapAnnouncement {
    pub header: SapAnnouncementHeader,
    pub sdp: sdp_rs::SessionDescription,
}

pub struct SapListener {
    rx: mpsc::Receiver<SapAnnouncement>,
    _task: tokio::task::JoinHandle<()>,
}

impl SapListener {
    pub fn bind(multicast_group: Ipv4Addr, interface: Ipv4Addr, port: u16) -> Aes67Result<Self> {
        let socket = std::net::UdpSocket::bind((interface, port))?;
        socket.join_multicast_v4(&multicast_group.into(), &interface)?;
        let socket = tokio::net::UdpSocket::from_std(socket)?;

        let (tx, rx) = mpsc::channel(64);
        let _task = tokio::spawn(async move {
            let mut buffer = [0u8; MAX_SAP_LENGTH];

            loop {
                let (len, _src) = match socket.recv_from(&mut buffer).await {
                    Ok((len, src)) => (len, src),
                    Err(e) => {
                        eprintln!("Error receiving SAP packet: {}", e);
                        continue;
                    }
                };

                let announcement = match Self::parse_sap_announcement(&buffer[..len]) {
                    Ok(announcement) => announcement,
                    Err(e) => {
                        eprintln!("Failed to parse SAP announcement: {}", e);
                        continue;
                    }
                };

                if let Err(_) = tx.send(announcement).await {
                    eprintln!("Receiver dropped, stopping SAP listener");
                    break;
                }
            }
        });

        Ok(Self { rx, _task })
    }

    fn parse_sap_announcement(data: &[u8]) -> Result<SapAnnouncement, Aes67Error> {
        if data.len() < 8 {
            return Err(Aes67Error::Sap("SAP packet too short".to_string()));
        }

        let mut reader: BufferReader = data.into();

        let flags = reader.read_u8()?;
        let version = (flags & 0b11100000) >> 5;
        let address_type = SapAnnouncementAddressType::from_u8((flags & 0b00010000) >> 4)?;
        // reserved bit is (data[0] & 0b00001000) >> 3
        let message_type = SapAnnouncementMessageType::from_u8((flags & 0b00000100) >> 2)?;
        let is_encrypted = (flags & 0b00000010) != 0;
        let is_compressed = (flags & 0b00000001) != 0;

        let authentication_length = reader.read_u8()?;
        let message_id_hash = reader.read_u16()?;

        let originating_source = match address_type {
            SapAnnouncementAddressType::Ipv4 => {
                let addr = Ipv4Addr::from_bits(reader.read_u32()?);
                IpAddr::V4(addr)
            }
            SapAnnouncementAddressType::Ipv6 => {
                let addr = std::net::Ipv6Addr::new(
                    reader.read_u16()?,
                    reader.read_u16()?,
                    reader.read_u16()?,
                    reader.read_u16()?,
                    reader.read_u16()?,
                    reader.read_u16()?,
                    reader.read_u16()?,
                    reader.read_u16()?,
                );
                IpAddr::V6(addr)
            }
        };

        if authentication_length != 0 || is_encrypted || is_compressed {
            return Err(Aes67Error::Sap(
                "Encrypted, compressed, or authenticated SAP packets are not supported".to_string(),
            ));
        }

        let payload_type = String::from_utf8_lossy_owned(reader.read_until(0)?);

        if payload_type != "application/sdp" {
            return Err(Aes67Error::Sap(format!(
                "Unsupported payload type: {}",
                payload_type
            )));
        }

        let header = SapAnnouncementHeader {
            version,
            address_type,
            message_type,
            is_encrypted,
            is_compressed,
            authentication_length,
            message_id_hash,
            originating_source,
            payload_type,
        };

        let payload = String::from_utf8_lossy(reader.bytes()).to_string();
        let sdp = sdp_rs::SessionDescription::try_from(payload)?;

        Ok(SapAnnouncement { header, sdp })
    }

    pub async fn recv(&mut self) -> Option<SapAnnouncement> {
        self.rx.recv().await
    }
}
