use portus::ipc;
use portus::Error;
use std::net::UdpSocket;

use super::Result;

pub struct Socket {
    sk: UdpSocket,
}

impl Socket {
    pub fn new(port: u16) -> Result<Self> {
        let sk = UdpSocket::bind(("127.0.0.1", port))?;
        Ok(Socket { sk })
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
        self.sk.recv(msg).map_err(Error::from)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
