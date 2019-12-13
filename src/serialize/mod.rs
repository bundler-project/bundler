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

pub const QDISC_FEEDBACK_MSG_TYPE: u32 = 1;

/// Netlink feedback from in-box qdisc.
#[derive(Clone, Debug, PartialEq)]
pub struct QDiscFeedbackMsg {
    pub msg_type: u32,
    pub bundle_id: u32,
    pub marked_packet_hash: u32,
    pub curr_qlen: u32,
    pub epoch_bytes: u64,
    pub epoch_time: u64,
}

impl QDiscFeedbackMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 4 * 4 + 2 * 8]; // 32 bytes
        LittleEndian::write_u32(&mut buf[0..4], self.msg_type);
        LittleEndian::write_u32(&mut buf[4..8], self.bundle_id);
        LittleEndian::write_u32(&mut buf[8..12], self.marked_packet_hash);
        LittleEndian::write_u32(&mut buf[12..16], self.curr_qlen);
        LittleEndian::write_u64(&mut buf[16..24], self.epoch_bytes);
        LittleEndian::write_u64(&mut buf[24..32], self.epoch_time);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        QDiscFeedbackMsg {
            msg_type: LittleEndian::read_u32(&buf[0..4]),
            bundle_id: LittleEndian::read_u32(&buf[4..8]),
            marked_packet_hash: LittleEndian::read_u32(&buf[8..12]),
            curr_qlen: LittleEndian::read_u32(&buf[12..16]),
            epoch_bytes: LittleEndian::read_u64(&buf[16..24]),
            epoch_time: LittleEndian::read_u64(&buf[24..32]),
        }
    }
}

pub const UPDATE_QDISC_MSG_TYPE: u32 = 2;

/// Netlink message requesting the qdisc to change the rate at which it samples packets for a given
/// bundle.
/// The rate is specified as the epoch length in number of packets.
#[derive(Clone, Debug, PartialEq)]
pub struct QDiscUpdateMsg {
    pub msg_type: u32,
    pub bundle_id: u32,
    pub sample_rate: u32,
}

impl QDiscUpdateMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 3 * 4]; // 12 bytes
        LittleEndian::write_u32(&mut buf[0..4], UPDATE_QDISC_MSG_TYPE);
        LittleEndian::write_u32(&mut buf[4..8], self.bundle_id);
        LittleEndian::write_u32(&mut buf[8..12], self.sample_rate);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        QDiscUpdateMsg {
            msg_type: UPDATE_QDISC_MSG_TYPE,
            bundle_id: LittleEndian::read_u32(&buf[4..8]),
            sample_rate: LittleEndian::read_u32(&buf[8..12]),
        }
    }
}
pub const UPDATE_PRIO_MSG_TYPE: u32 = 3;

/// Netlink message requesting the qdisc to set the priority of the given flow_id to the given
/// flow_prio.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdatePrioMsg {
    pub msg_type: u32,
    pub bundle_id: u32,
    pub flow_id: u32,
    pub flow_prio: u16,
}

impl UpdatePrioMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 4 * 4]; // 16 bytes
        LittleEndian::write_u32(&mut buf[0..4], UPDATE_PRIO_MSG_TYPE);
        LittleEndian::write_u32(&mut buf[4..8], self.bundle_id);
        LittleEndian::write_u32(&mut buf[8..12], self.flow_id);
        LittleEndian::write_u16(&mut buf[12..14], self.flow_prio);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        Self {
            msg_type: UPDATE_PRIO_MSG_TYPE,
            bundle_id: LittleEndian::read_u32(&buf[4..8]),
            flow_id: LittleEndian::read_u32(&buf[8..12]),
            flow_prio: LittleEndian::read_u16(&buf[12..14]),
        }
    }
}

pub const QDISC_PRIO_MSG_TYPE: u32 = 4;

/// Netlink message describing a new flow that a priority can be set for.
#[derive(Clone, Debug, PartialEq)]
pub struct QDiscPrioMsg {
    pub msg_type: u32,
    pub bundle_id: u32,
    pub flow_id: u32,
    pub src_ip: u32,
    pub src_port: u16,
    pub dst_ip: u32,
    pub dst_port: u16,
}

impl QDiscPrioMsg {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; 3 * 4]; // 16 bytes
        LittleEndian::write_u32(&mut buf[0..4], QDISC_PRIO_MSG_TYPE);
        LittleEndian::write_u32(&mut buf[4..8], self.bundle_id);
        LittleEndian::write_u32(&mut buf[8..12], self.flow_id);
        LittleEndian::write_u32(&mut buf[12..16], self.src_ip);
        LittleEndian::write_u16(&mut buf[16..18], self.src_port);
        LittleEndian::write_u32(&mut buf[18..22], self.dst_ip);
        LittleEndian::write_u16(&mut buf[22..24], self.dst_port);
        buf
    }

    pub fn from_slice(buf: &[u8]) -> Self {
        Self {
            msg_type: LittleEndian::read_u32(&buf[0..4]),
            bundle_id: LittleEndian::read_u32(&buf[4..8]),
            flow_id: LittleEndian::read_u32(&buf[8..12]),
            src_ip: LittleEndian::read_u32(&buf[12..16]),
            src_port: LittleEndian::read_u16(&buf[16..18]),
            dst_ip: LittleEndian::read_u32(&buf[18..22]),
            dst_port: LittleEndian::read_u16(&buf[22..24]),
        }
    }
}

pub enum QDiscRecvMsgs {
    BundleFeedback(QDiscFeedbackMsg),
    FlowPrio(QDiscPrioMsg),
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
            msg_type: super::QDISC_FEEDBACK_MSG_TYPE,
            bundle_id: 3,
            marked_packet_hash: 0x3fff_ffff,
            curr_qlen: 24,
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
            msg_type: super::UPDATE_QDISC_MSG_TYPE,
            bundle_id: 4,
            sample_rate: 128,
        };
        let buf = m.as_bytes();
        let ms = QDiscUpdateMsg::from_slice(&buf);
        assert_eq!(m, ms);
    }
}
