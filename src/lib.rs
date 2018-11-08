//! A Bundler CCP datapath.
//!
//! This is the sender side. Its responsibilities are to:
//! 1. communicate with the pacing qdisc
//! 2. communicate out-of-band with the receiver side of the virutal congestion tunnel
//! 3. enforce measurements and issue calls to libccp

extern crate bytes;
extern crate failure;
extern crate minion;
extern crate portus;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
mod libccp;

use portus::ipc;
use portus::ipc::netlink;
pub mod serialize;
pub mod udp;

use portus::Result;

pub struct Runtime {
    // netlink socket
    nlsk: netlink::Socket<ipc::Blocking>,
    // UDP socket
    udpsk: udp::Socket,
}

impl Runtime {
    pub fn new() -> Self {
        unimplemented!()
    }
}

impl minion::Cancellable for Runtime {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        // read from netlink socket?
        // read from udp socket?

        Ok(minion::LoopState::Continue)
    }
}
