use crate::inbox::ConnectionImpl;
use crate::marks::{Epoch, EpochHistory, MarkHistory, MarkedInstant};
use crate::serialize::OutBoxFeedbackMsg;
use slog::info;

/// Calculate and maintain flow measurements.
#[derive(Default)]
pub struct BundleFlowState<'dp> {
    pub conn: Option<libccp::Connection<'dp, ConnectionImpl>>,
    pub marked_packets: MarkHistory,
    pub epoch_history: EpochHistory,

    pub prev_send_time: u64,
    pub prev_send_byte_clock: u64,
    pub prev_recv_time: u64,
    pub prev_recv_byte_clock: u64,

    pub send_rate: f64,
    pub recv_rate: f64,
    pub rtt_estimate: u64,

    pub bdp_estimate_packets: u32,
    pub acked_bytes: u32, // estimate with number of received packets in last epoch
    pub lost_bytes: u32,

    pub curr_qlen: u32,
}

impl<'dp> BundleFlowState<'dp> {
    ///
    /// s1  |\   A       |
    ///     | -------    |
    ///     |    B   \   |
    /// s2  |\-----   ---| r1
    ///     |      \ /   |
    ///     | -------    |
    /// s1' |/   A'  \---| r2
    ///     |        /   |
    ///     | -------    |
    /// s2' |/   B'      |
    ///
    ///
    /// RTT = s2' - s2 = NOW - s2
    /// send epoch = s1 -> s2
    /// recv epoch = r1 -> r2
    /// r1 available with s1'
    /// r2 available with s2'
    ///
    /// We are currently at s2'.
    pub fn update_measurements(
        &mut self,
        now: u64,
        sent_mark: MarkedInstant,
        recv_mark: OutBoxFeedbackMsg,
        logger: &slog::Logger,
    ) {
        let s1 = self.prev_send_time;
        let s1_bytes = self.prev_send_byte_clock;
        let s2 = sent_mark.time;
        let s2_bytes = sent_mark.send_byte_clock;
        let r1 = self.prev_recv_time;
        let r1_bytes = self.prev_recv_byte_clock;
        let r2 = recv_mark.epoch_time;
        let r2_bytes = recv_mark.epoch_bytes;

        // rtt is current time - sent mark time
        self.rtt_estimate = now - s2;

        let send_epoch_ns = s2 - s1;
        let recv_epoch_ns = r2 - r1;

        let send_epoch_bytes = s2_bytes - s1_bytes;
        let recv_epoch_bytes = r2_bytes - r1_bytes;

        let (send_rate, recv_rate) = self.epoch_history.got_epoch(
            Epoch {
                elapsed_ns: send_epoch_ns,
                bytes: send_epoch_bytes,
            },
            Epoch {
                elapsed_ns: recv_epoch_ns,
                bytes: recv_epoch_bytes,
            },
        );
        //let send_rate = send_epoch_bytes as f64 / (send_epoch_ns as f64 / 1e9);
        //let recv_rate = recv_epoch_bytes as f64 / (recv_epoch_ns as f64 / 1e9);

        self.send_rate = send_rate;
        self.recv_rate = recv_rate;

        info!(logger, "epochs";
            "send_epoch_bytes" => send_epoch_bytes,
            "send_epoch_ns" => send_epoch_ns,
            "recv_epoch_bytes" => recv_epoch_bytes,
            "recv_epoch_ns" => recv_epoch_ns,
            "epoch_window" => self.epoch_history.window,
        );

        let rtt_s = self.rtt_estimate as f64 / 1e9;
        let bdp_estimate_bytes = send_rate as f64 * rtt_s;
        self.bdp_estimate_packets = (bdp_estimate_bytes / 1514.0) as u32;
        self.acked_bytes = recv_epoch_bytes as u32;
        let delta = send_epoch_bytes.saturating_sub(recv_epoch_bytes);
        self.lost_bytes = if delta > 0 { delta as u32 } else { 0 };

        // s2 now becomes s1 and r2 becomes r1
        self.prev_send_time = s2;
        self.prev_send_byte_clock = s2_bytes;
        self.prev_recv_time = r2;
        self.prev_recv_byte_clock = r2_bytes;

        self.update_primitives()
    }

    pub fn did_invoke(&mut self) {
        self.acked_bytes = 0;
        self.lost_bytes = 0;
        self.update_primitives()
    }

    fn update_primitives(&mut self) {
        // set primitives
        if let Some(c) = self.conn.as_mut() {
            c.load_primitives(
                libccp::Primitives::default()
                    .with_rate_outgoing(self.send_rate as u64)
                    .with_rate_incoming(self.recv_rate as u64)
                    .with_rtt_sample_us(self.rtt_estimate / 1_000)
                    .with_bytes_acked(self.acked_bytes)
                    .with_packets_acked(self.acked_bytes / 1514)
                    .with_lost_pkts_sample(self.lost_bytes / 1514)
                    .with_bytes_pending(self.curr_qlen), // quick hack
            );
        }
    }
}
