//! A Bundler CCP datapath.
//!
//!
//! This is the sender side. Its responsibilities are to:
//! 1. communicate with the pacing qdisc
//! 2. communicate out-of-band with the receiver side of the virutal congestion tunnel
//! 3. enforce measurements and issue calls to libccp

extern crate bytes;
extern crate failure;
extern crate minion;
extern crate portus;

use portus::ipc;
use portus::ipc::netlink;
use portus::Result;

pub mod serialize;
pub mod udp;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
mod ccp;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
mod nl;

pub mod qdisc;
use self::qdisc::*;

pub struct Runtime {
    // netlink socket
    nlsk: netlink::Socket<ipc::Blocking>,
    // UDP socket
    udpsk: udp::Socket,
    // qdisc handle
    qdisc: Qdisc,
}

impl Runtime {
    pub fn new() -> Self {
        // TODO For now assumes root qdisc on the 10gp1 interface, but this
        // should be configurable  or we should add a deterministic way to
        // discover it correctly.
        let _qdisc = Qdisc::get(String::from("10gp1"), (1, 0));

        //Runtime { qdisc }
        unimplemented!()
    }
}

impl minion::Cancellable for Runtime {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        // read from netlink socket?
        // read from udp socket?
        let rate = 0;
        let burst = 0;
        self.qdisc.set_rate(rate, burst);

        Ok(minion::LoopState::Continue)
    }
}
