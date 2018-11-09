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
