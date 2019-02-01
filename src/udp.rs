use portus::ipc;
use portus::Error;
use std::net::UdpSocket;
use std::sync::mpsc;

use super::Result;

pub struct Socket {
    sk: UdpSocket,
    outbox_found: mpsc::Sender<std::net::SocketAddr>,
}

impl Socket {
    pub fn new(port: u16, outbox_found: mpsc::Sender<std::net::SocketAddr>) -> Result<Self> {
        let sk = UdpSocket::bind(("0.0.0.0", port))?;
        Ok(Socket { sk, outbox_found })
    }

    pub fn try_clone(&self) -> UdpSocket {
        self.sk.try_clone().unwrap()
    }
}

impl ipc::Ipc for Socket {
    fn name() -> String {
        String::from("udp")
    }

    fn send(&self, _msg: &[u8]) -> Result<()> {
        unimplemented!()
    }

    fn recv(&self, msg: &mut [u8]) -> Result<usize> {
        self.sk
            .recv_from(msg)
            .map(|(bytes, from)| {
                self.outbox_found.send(from).unwrap_or_else(|_| ());
                bytes
            })
            .map_err(Error::from)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
