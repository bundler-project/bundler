use bytes::{ByteOrder, LittleEndian};

/// UDP out-of-band feedback from the out-box.
/// Receive time of this message, combined with the
/// send time of the corresponding marked packet, gets us
/// RTT.
#[derive(Clone, Debug, PartialEq)]
pub struct OutBoxFeedbackMsg {
    bundle_id: u32,
    epoch_bytes: u32,
    marked_packet_hash: u32,
    recv_time: u64,
    epoch_time: u64,
}

impl OutBoxFeedbackMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 3 * 4 + 2 * 8]; // 28 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.bundle_id);
        LittleEndian::write_u32(&mut buf[4..8], self.epoch_bytes);
        LittleEndian::write_u32(&mut buf[8..12], self.marked_packet_hash);
        LittleEndian::write_u64(&mut buf[12..20], self.recv_time);
        LittleEndian::write_u64(&mut buf[20..28], self.epoch_time);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        OutBoxFeedbackMsg {
            bundle_id: LittleEndian::read_u32(&buf[0..4]),
            epoch_bytes: LittleEndian::read_u32(&buf[4..8]),
            marked_packet_hash: LittleEndian::read_u32(&buf[8..12]),
            recv_time: LittleEndian::read_u64(&buf[12..20]),
            epoch_time: LittleEndian::read_u64(&buf[20..28]),
        }
    }
}

/// Netlink feedback from in-box qdisc.
#[derive(Clone, Debug, PartialEq)]
pub struct QDiscFeedbackMsg {
    bundle_id: u32,
    epoch_bytes: u32,
    marked_packet_hash: u32,
    epoch_time: u64,
}

impl QDiscFeedbackMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 3 * 4 + 1 * 8]; // 20 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.bundle_id);
        LittleEndian::write_u32(&mut buf[4..8], self.epoch_bytes);
        LittleEndian::write_u32(&mut buf[8..12], self.marked_packet_hash);
        LittleEndian::write_u64(&mut buf[12..20], self.epoch_time);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        QDiscFeedbackMsg {
            bundle_id: LittleEndian::read_u32(&buf[0..4]),
            epoch_bytes: LittleEndian::read_u32(&buf[4..8]),
            marked_packet_hash: LittleEndian::read_u32(&buf[8..12]),
            epoch_time: LittleEndian::read_u64(&buf[12..20]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OutBoxFeedbackMsg, QDiscFeedbackMsg};

    #[test]
    fn check_outbox_msg() {
        let m = OutBoxFeedbackMsg {
            bundle_id: 3,
            epoch_bytes: 0xe,
            marked_packet_hash: 0x3fff_ffff,
            recv_time: 0xf0f0_f0f0,
            epoch_time: 0x3030_3030,
        };

        let buf = m.as_bytes();
        let ms = OutBoxFeedbackMsg::from_slice(&buf);
        assert_eq!(m, ms);
    }

    #[test]
    fn check_qdisc_msg() {
        let m = QDiscFeedbackMsg {
            bundle_id: 3,
            epoch_bytes: 0xe,
            marked_packet_hash: 0x3fff_ffff,
            epoch_time: 0x3030_3030,
        };

        let buf = m.as_bytes();
        let ms = QDiscFeedbackMsg::from_slice(&buf);
        assert_eq!(m, ms);
    }
}
