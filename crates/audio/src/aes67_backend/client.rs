use std::{collections::HashMap, net::Ipv4Addr};

use crate::aes67_backend::{
    error::Aes67Result,
    rtp::{RtpReceiver, RtpReceiverConfig},
    sap::{SapAnnouncement, SapListener},
};

pub struct Client {
    receivers: HashMap<String, tokio::task::JoinHandle<()>>,
    sap: SapListener,

    interface: Ipv4Addr,
}

impl Client {
    pub fn new(sap_multicast_group: Ipv4Addr, interface: Ipv4Addr, port: u16) -> Aes67Result<Self> {
        let sap = SapListener::bind(sap_multicast_group, interface, port)?;
        Ok(Self {
            sap,
            receivers: HashMap::new(),
            interface,
        })
    }

    pub async fn start(&mut self) -> Aes67Result<()> {
        while let Some(announcement) = self.sap.recv().await {
            let _ = self.handle_announcement(announcement);
        }

        Ok(())
    }
}

impl Client {
    fn handle_announcement(&mut self, ann: SapAnnouncement) -> Aes67Result<()> {
        let mut sdp = ann.sdp;
        if self.receivers.contains_key(sdp.session_name.value()) {
            return Ok(());
        }

        let media_description = sdp.media_descriptions.pop().unwrap();
        let interface = self.interface;

        let handle = tokio::spawn(async move {
            let config = RtpReceiverConfig {
                media_description,
                interface,
            };

            let _receiver = match RtpReceiver::spawn(config) {
                Ok(receiver) => receiver,
                Err(err) => {
                    eprintln!("Failed to spawn RTP receiver: {}", err);
                    return;
                }
            };
        });

        self.receivers.insert(sdp.session_name.to_string(), handle);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use crate::aes67_backend::client::Client;

    #[test]
    fn hello_world() {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut client = Client::new(
                    Ipv4Addr::new(239, 255, 255, 255),
                    Ipv4Addr::new(192, 168, 2, 100),
                    9875,
                )
                .unwrap();

                let _ = client.start().await;
            });
    }
}
