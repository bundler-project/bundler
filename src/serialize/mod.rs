use bytes::{ByteOrder, LittleEndian};

#[derive(Clone, Debug, PartialEq)]
pub struct OutBoxReportMsg {
    pub bundle_id: u32,
    pub epoch_length_packets: u32,
}

impl OutBoxReportMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 2 * 4]; // 8 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.bundle_id);
        LittleEndian::write_u32(&mut buf[4..8], self.epoch_length_packets);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        OutBoxReportMsg {
            bundle_id: LittleEndian::read_u32(&buf[0..4]),
            epoch_length_packets: LittleEndian::read_u32(&buf[4..8]),
        }
    }
}

/// UDP out-of-band feedback from the out-box.
/// Receive time of this message, combined with the
/// send time of the corresponding marked packet, gets us
/// RTT.
#[derive(Clone, Debug, PartialEq)]
pub struct OutBoxFeedbackMsg {
    pub bundle_id: u32,
    pub marked_packet_hash: u32,
    pub epoch_bytes: u64,
    pub epoch_time: u64,
}

impl OutBoxFeedbackMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 2 * 4 + 2 * 8]; // 24 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.bundle_id);
        LittleEndian::write_u32(&mut buf[4..8], self.marked_packet_hash);
        LittleEndian::write_u64(&mut buf[8..16], self.epoch_bytes);
        LittleEndian::write_u64(&mut buf[16..24], self.epoch_time);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        OutBoxFeedbackMsg {
            bundle_id: LittleEndian::read_u32(&buf[0..4]),
            marked_packet_hash: LittleEndian::read_u32(&buf[4..8]),
            epoch_bytes: LittleEndian::read_u64(&buf[8..16]),
            epoch_time: LittleEndian::read_u64(&buf[16..24]),
        }
    }
}

/// Netlink feedback from in-box qdisc.
#[derive(Clone, Debug, PartialEq)]
pub struct QDiscFeedbackMsg {
    pub bundle_id: u32,
    pub marked_packet_hash: u32,
    pub epoch_bytes: u64,
    pub epoch_time: u64,
}

impl QDiscFeedbackMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 2 * 4 + 2 * 8]; // 24 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.bundle_id);
        LittleEndian::write_u32(&mut buf[4..8], self.marked_packet_hash);
        LittleEndian::write_u64(&mut buf[8..16], self.epoch_bytes);
        LittleEndian::write_u64(&mut buf[16..24], self.epoch_time);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        QDiscFeedbackMsg {
            bundle_id: LittleEndian::read_u32(&buf[0..4]),
            marked_packet_hash: LittleEndian::read_u32(&buf[4..8]),
            epoch_bytes: LittleEndian::read_u64(&buf[8..16]),
            epoch_time: LittleEndian::read_u64(&buf[16..24]),
        }
    }
}

/// Netlink message requesting the qdisc to change the rate at which it samples packets for a given
/// bundle.
/// The rate is specified as the epoch length in number of packets.
#[derive(Clone, Debug, PartialEq)]
pub struct QDiscUpdateMsg {
    pub bundle_id: u32,
    pub sample_rate: u32,
}

impl QDiscUpdateMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 2 * 4]; // 8 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.bundle_id);
        LittleEndian::write_u32(&mut buf[4..8], self.sample_rate);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        QDiscUpdateMsg {
            bundle_id: LittleEndian::read_u32(&buf[0..4]),
            sample_rate: LittleEndian::read_u32(&buf[4..8]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OutBoxFeedbackMsg, QDiscFeedbackMsg, QDiscUpdateMsg};

    #[test]
    fn check_outbox_msg() {
        let m = OutBoxFeedbackMsg {
            bundle_id: 3,
            marked_packet_hash: 0x3fff_ffff,
            epoch_bytes: 0xe,
            epoch_time: 0xf0f0_f0f0,
        };

        let buf = m.as_bytes();
        let ms = OutBoxFeedbackMsg::from_slice(&buf);
        assert_eq!(m, ms);
    }

    #[test]
    fn check_qdisc_msg() {
        let m = QDiscFeedbackMsg {
            bundle_id: 3,
            marked_packet_hash: 0x3fff_ffff,
            epoch_bytes: 0xe,
            epoch_time: 0x3030_3030,
        };

        let buf = m.as_bytes();
        let ms = QDiscFeedbackMsg::from_slice(&buf);
        assert_eq!(m, ms);
    }

    #[test]
    fn check_update_msg() {
        let m = QDiscUpdateMsg {
            bundle_id: 4,
            sample_rate: 128,
        };
        let buf = m.as_bytes();
        let ms = QDiscUpdateMsg::from_slice(&buf);
        assert_eq!(m, ms);
    }
}
