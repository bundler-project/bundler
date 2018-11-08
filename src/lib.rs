//! A Bundler CCP datapath.
//!
//! This is the sender side. Its responsibilities are to:
//! 1. communicate with the pacing qdisc
//! 2. communicate out-of-band with the receiver side of the virutal congestion tunnel
//! 3. enforce measurements and issue calls to libccp

// libccp rust bindings
include!(concat!(env!("OUT_DIR"), "/libccp.rs"));

extern crate failure;
extern crate minion;
extern crate portus;

use portus::ipc;
use portus::ipc::netlink;
pub mod udp;
use portus::Result;
