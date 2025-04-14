use super::ProxyStream;

use tokio::io::AsyncReadExt;
use worker::*;

impl <'a> ProxyStream<'a> {
    pub async fn process_shadowsocks(&mut self) -> Result<()> {
        // read port and address
        let remote_addr = crate::common::parse_addr(self).await?;
        let remote_port = {
            let mut port = [0u8; 2];
            self.read_exact(&mut port).await?;
            ((port[0] as u16) << 8) | (port[1] as u16)
        };
        
        let is_tcp = true; // difficult to detect udp packet from shadowsocks
        console_log!("connecting to upstream {}:{} [is_tcp={is_tcp}]", remote_addr, remote_port);

        if is_tcp {
            let addr_pool = [
                (remote_addr.clone(), remote_port),
                (self.config.proxy_addr.clone(), self.config.proxy_port)
            ];

            // send header
            for (target_addr, target_port) in addr_pool {
                if let Err(e) = self.handle_tcp_outbound(target_addr, target_port).await {
                    console_error!("error handling tcp: {}", e)
                }
            }
        } else {
            if let Err(e) = self.handle_udp_outbound().await {
                console_error!("error handling udp: {}", e)
            }
        }

        Ok(())
    }
}