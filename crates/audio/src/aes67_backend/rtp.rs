use std::net::Ipv4Addr;

use sdp_rs::lines::Attribute;
use tokio::sync::mpsc;

use crate::aes67_backend::{error::Aes67Result, util::MediaDescriptionExt};

#[derive(Debug)]
pub struct RtpReceiverConfig {
    /// The interface to bind the UDP socket to.
    pub interface: Ipv4Addr,

    /// The media description from the SDP announcement.
    pub media_description: sdp_rs::MediaDescription,
}

impl RtpReceiverConfig {
    fn find_mulitcast_group(&self) -> Option<&Ipv4Addr> {
        self.media_description
            .connections
            .iter()
            .map(|c| &c.connection_address.base)
            .find_map(|c| {
                if let std::net::IpAddr::V4(ipv4) = c {
                    Some(ipv4)
                } else {
                    None
                }
            })
    }
}

pub struct RtpReceiver {
    rx: mpsc::Receiver<()>,
    _task: tokio::task::JoinHandle<()>,
}

impl RtpReceiver {
    pub fn spawn(config: RtpReceiverConfig) -> Aes67Result<Self> {
        let rtp_multicast_group = config.find_mulitcast_group().cloned().ok_or_else(|| {
            crate::aes67_backend::error::Aes67Error::RtpReceiver(
                "No multicast group found in media description".to_string(),
            )
        })?;
        let rtp_port = config.media_description.media.port;
        let frame_count = config
            .media_description
            .find_other_attribute("framecount")
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| {
                crate::aes67_backend::error::Aes67Error::RtpReceiver(
                    "No framecount attribute found in media description".to_string(),
                )
            })?;

        let rtp_map = config
            .media_description
            .attributes
            .into_iter()
            .find_map(|attr| {
                if let Attribute::Rtpmap(rtpmap) = attr {
                    Some(rtpmap)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                crate::aes67_backend::error::Aes67Error::RtpReceiver(
                    "No rtpmap attribute found in media description".to_string(),
                )
            })?;

        println!("joining multicast {}:{}", rtp_multicast_group, rtp_port);

        let socket = std::net::UdpSocket::bind((config.interface, rtp_port))?;
        socket.join_multicast_v4(&rtp_multicast_group, &config.interface)?;
        let socket = tokio::net::UdpSocket::from_std(socket)?;

        let (tx, rx) = mpsc::channel(64);
        let _task = tokio::spawn(async move {
            let mut buffer = [0u8; 2048];

            loop {
                let (len, _src) = match socket.recv_from(&mut buffer).await {
                    Ok((len, src)) => (len, src),
                    Err(e) => {
                        eprintln!("Error receiving RTP packet: {}", e);
                        continue;
                    }
                };

                if tx.send(()).await.is_err() {
                    eprintln!("Receiver dropped, stopping RTP listener");
                    break;
                }
            }
        });

        Ok(Self { _task, rx })
    }
}
