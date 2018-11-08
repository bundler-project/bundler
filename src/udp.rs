use portus::ipc;
use portus::Error;
use std::net::UdpSocket;

use super::Result;

pub struct Socket {
    sk: UdpSocket,
    dest: String,
}

impl Socket {
    pub fn new(port: u16, dest: String) -> Result<Self> {
        let sk = UdpSocket::bind(("127.0.0.1", port))?;
        Ok(Socket { sk, dest })
    }
}

impl ipc::Ipc for Socket {
    fn name() -> String {
        String::from("udp")
    }

    fn send(&self, msg: &[u8]) -> Result<()> {
        self.sk
            .send_to(msg, self.dest.clone())
            .map(|_| ())
            .map_err(Error::from)
    }

    fn recv(&self, msg: &mut [u8]) -> Result<usize> {
        self.sk.recv(msg).map_err(Error::from)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
