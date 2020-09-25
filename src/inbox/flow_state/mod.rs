use crate::inbox::ConnectionImpl;
use crate::serialize::OutBoxFeedbackMsg;
use slog::{debug, info};

mod marks;
use self::marks::{Epoch, EpochHistory, MarkHistory, MarkedInstant};

pub struct LibccpConn<'dp, Q: crate::inbox::datapath::Datapath + 'static>(
    libccp::Connection<'dp, ConnectionImpl<Q>>,
);

impl<'dp, Q: crate::inbox::datapath::Datapath> std::ops::Deref for LibccpConn<'dp, Q> {
    type Target = libccp::Connection<'dp, ConnectionImpl<Q>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<'dp, Q: crate::inbox::datapath::Datapath> std::ops::DerefMut for LibccpConn<'dp, Q> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'dp, Q: crate::inbox::datapath::Datapath> LibccpConn<'dp, Q> {
    pub fn new(c: libccp::Connection<'dp, ConnectionImpl<Q>>) -> Self {
        Self(c)
    }

    pub fn did_invoke(&mut self, fs: &BundleFlowState) {
        // set primitives
        self.0.load_primitives(
            libccp::Primitives::default()
                .with_rate_outgoing(fs.send_rate as u64)
                .with_rate_incoming(fs.recv_rate as u64)
                .with_rtt_sample_us(fs.rtt_estimate / 1_000)
                .with_bytes_acked(fs.acked_bytes)
                .with_packets_acked(fs.acked_bytes / 1514)
                .with_lost_pkts_sample(fs.lost_bytes / 1514)
                .with_bytes_pending(fs.curr_qlen), // quick hack
        );
    }
}

/// Calculate and maintain flow measurements.
#[derive(Debug, Clone)]
pub struct BundleFlowState {
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

    pub last_id: usize,
}

impl Default for BundleFlowState {
    fn default() -> Self {
        BundleFlowState {
            marked_packets: Default::default(),
            epoch_history: Default::default(),
            prev_send_time: Default::default(),
            prev_send_byte_clock: Default::default(),
            prev_recv_time: Default::default(),
            prev_recv_byte_clock: Default::default(),
            send_rate: Default::default(),
            recv_rate: Default::default(),
            rtt_estimate: Default::default(),
            bdp_estimate_packets: Default::default(),
            acked_bytes: Default::default(),
            lost_bytes: Default::default(),
            curr_qlen: Default::default(),
            last_id: Default::default(),
        }
    }
}

impl BundleFlowState {
    //pub fn new_measurements(
    //    self,
    //    now: u64,
    //    sent_mark: MarkedInstant,
    //    recv_mark: OutBoxFeedbackMsg,
    //    logger: &slog::Logger,
    //) -> Self {
    //    let mut new = self.clone();
    //    new.update_measurements(now, sent_mark, recv_mark, logger);
    //    new
    //}

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
        let rtt_estimate = now.saturating_sub(s2);

        let send_epoch_ns: u64 = s2.saturating_sub(s1);
        let recv_epoch_ns: u64 = r2.saturating_sub(r1);

        let send_epoch_bytes: u64 = s2_bytes.saturating_sub(s1_bytes);
        let recv_epoch_bytes: u64 = r2_bytes.saturating_sub(r1_bytes);

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

        //info!(logger, "epochs";
        //    "send_epoch_bytes" => send_epoch_bytes,
        //    "send_epoch_ns" => send_epoch_ns,
        //    "recv_epoch_bytes" => recv_epoch_bytes,
        //    "recv_epoch_ns" => recv_epoch_ns,
        //    "epoch_window" => self.epoch_history.window,
        //);
        let rtt_s = rtt_estimate as f64 / 1e9;
        let bdp_estimate_bytes = send_rate as f64 * rtt_s;
        let delta = send_epoch_bytes.saturating_sub(recv_epoch_bytes);

        self.send_rate = send_rate;
        self.recv_rate = recv_rate;

        self.bdp_estimate_packets = (bdp_estimate_bytes / 1514.0) as u32;
        self.acked_bytes = recv_epoch_bytes as u32;
        self.lost_bytes = if delta > 0 { delta as u32 } else { 0 };

        self.rtt_estimate = rtt_estimate;
        // s2 now becomes s1 and r2 becomes r1
        self.prev_send_time = s2;
        self.prev_send_byte_clock = s2_bytes;
        self.prev_recv_time = r2;
        self.prev_recv_byte_clock = r2_bytes;

        self.last_id = sent_mark.id;

        info!(logger, "ooo measurements";
            "now" => now,
            "hash" => sent_mark.pkt_hash,
            "id" => sent_mark.id,
            "prev_id" => self.last_id,
            "rtt" => rtt_estimate / 1_000,
            "rate_outgoing" => send_rate as u64,
            "rate_incoming" => recv_rate as u64,
        );
    }

    pub fn did_invoke(&mut self) {
        self.acked_bytes = 0;
        self.lost_bytes = 0;
    }
}
