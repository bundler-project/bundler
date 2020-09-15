//! A Bundler CCP datapath.

extern crate bytes;
extern crate crossbeam;
extern crate failure;
extern crate libccp;
extern crate minion;
extern crate portus;
extern crate slog;

pub mod hash;
pub mod inbox;
pub mod outbox;
pub mod serialize;

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

#[cfg(test)]
mod tests {
    #[test]
    fn test_round() {
        let x: f64 = 2.0 * 270.0;
        assert_eq!(crate::round_down_power_of_2(x as u32), 512);
        assert_eq!(crate::round_down_power_of_2(538), 512);
        assert_eq!(crate::round_down_power_of_2(16), 16);
        assert_eq!(crate::round_down_power_of_2(1), 1);
        assert_eq!(crate::round_down_power_of_2(0), 0);
    }
}
