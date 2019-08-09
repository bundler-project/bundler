//! A Bundler CCP datapath.
//!
//!
//! This is the sender side. Its responsibilities are to:
//! 1. communicate with the pacing qdisc
//! 2. communicate out-of-band with the receiver side of the virutal congestion tunnel
//! 3. enforce measurements and issue calls to libccp

extern crate bytes;
extern crate crossbeam;
extern crate failure;
extern crate libccp;
extern crate minion;
extern crate portus;
extern crate slog;

use portus::Result;

mod flow_state;
pub mod hash;
pub mod inbox;
mod marks;
mod nl;
pub mod outbox;
pub mod qdisc;
mod readers;
pub mod serialize;
pub mod udp;

// Header lengths
pub const MAC_HEADER_LENGTH: usize = 14;
pub const IP_HEADER_LENGTH: usize = 20;
// Locations in headers
pub const PROTO_IN_IP_HEADER: usize = 9;
// Values
pub const IP_PROTO_TCP: u8 = 6;

fn round_down_power_of_2(x: u32) -> u32 {
    let y = x.leading_zeros();
    if y >= 32 {
        0
    } else {
        1 << (32 - y - 1)
    }
}
