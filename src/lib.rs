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

use crossbeam::select;
use minion::Cancellable;
use portus::Result;
use slog::{debug, info};
use std::os::unix::net::UnixDatagram;

pub mod serialize;
use self::serialize::{OutBoxFeedbackMsg, QDiscFeedbackMsg};
pub mod udp;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(unused)]
mod nl;

pub mod qdisc;
use self::qdisc::*;

mod marks;
use self::marks::{Epoch, EpochHistory, MarkHistory};

mod readers;
use self::readers::{NlMsgReader, UdpMsgReader, UnixMsgReader};

struct DatapathImpl {
    sk: UnixDatagram,
    connected: bool,
}

impl libccp::DatapathOps for DatapathImpl {
    fn send_msg(&mut self, msg: &[u8]) {
        // construct the slice
        match self.sk.send_to(msg, "/tmp/ccp/0/in") {
            Err(ref e)
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if self.connected {
                    eprintln!("warn: unix socket does not exist...");
                }
                self.connected = false;
                Ok(())
            }
            Err(e) => Err(e),
            Ok(_) => {
                if !self.connected {
                    eprintln!("info: unix socket connected!");
                }
                self.connected = true;
                Ok(())
            }
        }
        .unwrap();
    }
}

struct ConnectionImpl {
    qdisc: Rc<RefCell<Qdisc>>, // qdisc handle
}

impl libccp::CongestionOps for ConnectionImpl {
    fn set_cwnd(&mut self, cwnd: u32) {
        let set = if cwnd == 0 { 15_000 } else { cwnd };

        self.qdisc
            .borrow_mut()
            .set_approx_cwnd(set)
            .unwrap_or_else(|_| ())
    }

    fn set_rate_abs(&mut self, rate: u32) {
        self.qdisc
            .borrow_mut()
            .set_rate(rate)
            .unwrap_or_else(|_| ())
    }
}

/// Calculate and maintain flow measurements.
#[derive(Default)]
struct BundleFlowState<'dp> {
    conn: Option<libccp::Connection<'dp, ConnectionImpl>>,
    marked_packets: MarkHistory,
    epoch_history: EpochHistory,

    prev_send_time: u64,
    prev_send_byte_clock: u64,
    prev_recv_time: u64,
    prev_recv_byte_clock: u64,

    send_rate: f64,
    recv_rate: f64,
    rtt_estimate: u64,

    bdp_estimate_packets: u32,
    acked_bytes: u32, // estimate with number of received packets in last epoch
    lost_bytes: u32,

    curr_qlen: u32,
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
    fn update_measurements(
        &mut self,
        now: u64,
        sent_mark: marks::MarkedInstant,
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

    fn did_invoke(&mut self) {
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

use crossbeam::tick;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Runtime {
    log: slog::Logger,
    qdisc_recv: crossbeam::Receiver<QDiscFeedbackMsg>,
    outbox_recv: crossbeam::Receiver<OutBoxFeedbackMsg>,
    // Do not allow a reference to flow_state to escape.
    // Doing so would be unsafe, because the lifetime is
    // declared as 'static when it is actually the same
    // as the lifetime of Arc<libccp::Datapath>.
    flow_state: BundleFlowState<'static>,
    qdisc: Rc<RefCell<Qdisc>>,
    invoke_ticker: crossbeam::Receiver<Instant>,
    ready_to_invoke: bool,
    // Must come last. Since Drop on libccp::Datapath frees
    // libccp state, if Runtime is ever Dropped then this must
    // be dropped last.
    // See https://github.com/rust-lang/rfcs/blob/master/text/1857-stabilize-drop-order.md
    // for drop order documentation.
    datapath: Arc<libccp::Datapath>,
}

// This prevents `Runtime` from being destructured, which could cause `flow_state` to escape.
impl Drop for Runtime {
    fn drop(&mut self) {}
}

impl Runtime {
    pub fn new(
        log: slog::Logger,
        listen_port: u16,
        outbox: Option<String>,
        iface: String,
        handle: (u32, u32),
        use_dynamic_epoch: bool,
        sample_freq: u32,
    ) -> Option<Self> {
        use portus::ipc;
        use portus::ipc::netlink;

        let nlsk = netlink::Socket::<ipc::Blocking>::new().unwrap();
        let (qdisc_reader, qdisc_recv) = NlMsgReader::make(nlsk);
        let _qdisc_recv_handle = qdisc_reader.spawn();

        let (outbox_found_tx, outbox_found_rx) = std::sync::mpsc::channel();
        if let Some(to) = outbox {
            use std::net::ToSocketAddrs;
            outbox_found_tx
                .send(to.to_socket_addrs().unwrap().next().unwrap())
                .unwrap_or_else(|_| ());
        }

        let udpsk = udp::Socket::new(listen_port, outbox_found_tx).unwrap();
        // udp socket for sending *to* outbox
        let outbox_report = udpsk.try_clone();

        let (outbox_reader, outbox_recv) = UdpMsgReader::make(udpsk);
        let _outbox_recv_handle = outbox_reader.spawn();

        // unix socket for sending *to* portus
        let portus_sk = UnixDatagram::unbound().unwrap();

        let mut qdisc = Qdisc::bind(
            log.clone(),
            iface,
            handle,
            use_dynamic_epoch,
            outbox_found_rx,
            outbox_report,
        );

        qdisc.set_epoch_length(sample_freq).unwrap_or_else(|_| ());

        let qdisc = Rc::new(RefCell::new(qdisc));

        let dpi = DatapathImpl {
            sk: portus_sk,
            connected: true,
        };

        let dp = libccp::Datapath::init(dpi).unwrap();
        let dp = Arc::new(dp);

        let (portus_reader, alg_ready) = UnixMsgReader::make(log.clone(), dp.clone());
        let _portus_reader_handle = portus_reader.spawn();

        // Wait for algorithm to finish installing datapath programs
        info!(log, "Wait for CCP to install datapath program");
        alg_ready.recv().unwrap();

        info!(log, "Initialize bundle flow in libccp");
        // TODO this is a hack, we are pretending there is only one bundle/flow
        let dp_info = libccp::FlowInfo::default()
            .with_init_cwnd(15_000)
            .with_mss(1514)
            .with_four_tuple(0, 0, 0, 0);

        // Why the mem::transmute you ask?
        // This is necessary because the correct lifetime is *self-referential*.
        // It is safe in this case because:
        // (1) libccp::Datapath::init(/*..*/) is inside an Arc, and at least one copy of that Arc is inside Runtime
        // (2) this libccp::Connection is inside BundleFlowState, which is also inside Runtime.
        // (3) Therefore, libccp::Connection is valid for the lifetime of Runtime, which is
        // effectively 'static.
        let conn = libccp::Connection::start(
            unsafe { std::mem::transmute(dp.as_ref()) },
            ConnectionImpl {
                qdisc: qdisc.clone(),
            },
            dp_info,
        )
        .unwrap();

        let mut fs: BundleFlowState = Default::default();
        fs.conn = Some(conn);
        fs.epoch_history.window = 1;

        let invoke_ticker = tick(Duration::from_millis(10));

        info!(log, "Inbox ready");
        Some(Runtime {
            log,
            qdisc_recv,
            outbox_recv,
            flow_state: fs,
            qdisc,
            invoke_ticker,
            ready_to_invoke: false,
            datapath: dp,
        })
    }
}

impl minion::Cancellable for Runtime {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        select! {
            recv(self.qdisc_recv) -> msg => {
                if let Ok(msg) = msg {
                    // remember the marked packet's send time
                    // so we can get its RTT later
                    // TODO -- this might need to get the current time instead of using the
                    // kernel's
                    debug!(self.log, "inbox epoch";
                        "time" => msg.epoch_time,
                        "hash" => msg.marked_packet_hash,
                        "bytes" => msg.epoch_bytes,
                        "curr_qlen" => msg.curr_qlen,
                    );

                    self.flow_state.marked_packets.insert(msg.marked_packet_hash, msg.epoch_time, msg.epoch_bytes);
                    self.flow_state.curr_qlen = msg.curr_qlen;
                }
            },
            recv(self.outbox_recv) -> msg => {
                if let Ok(msg) = msg {
                    // check packet marking
                    let now = time::precise_time_ns();
                    if let Some(mi) = self.flow_state.marked_packets.get(now, msg.marked_packet_hash) {
                        let h = msg.marked_packet_hash;
                        self.flow_state.update_measurements(now, mi, msg, &self.log);
                        {
                            let mut q = self.qdisc.borrow_mut();
                            q.update_rtt(self.flow_state.rtt_estimate).unwrap_or_else(|_| ());
                            q.update_send_rate(self.flow_state.send_rate as u64);
                        }

                        debug!(self.log, "new measurements";
                            "now" => now,
                            "hash" => h,
                            "rtt" => self.flow_state.rtt_estimate / 1_000,
                            "rate_outgoing" => self.flow_state.send_rate as u64,
                            "rate_incoming" => self.flow_state.recv_rate as u64,
                        );

                        self.ready_to_invoke = true;
                    } else {
                        debug!(self.log, "no match";
                            "hash" => msg.marked_packet_hash,
                        );
                    }
                }
            },
            recv(self.invoke_ticker) -> _ => {
                if self.ready_to_invoke {
                    let conn = self.flow_state.conn.as_mut().unwrap();

                    let prims = conn.primitives(&self.datapath);
                    info!(self.log, "CCP Invoke";
                          "rtt" => prims.0.rtt_sample_us,
                          "rate_outgoing" => prims.0.rate_outgoing,
                          "rate_incoming" => prims.0.rate_incoming,
                          "acked" => prims.0.packets_acked,
                          "lost_pkts_sample" => prims.0.lost_pkts_sample,
                    );

                    // ccp_invoke
                    conn.invoke().unwrap_or_else(|_| ());

                    // reset measurements
                    self.flow_state.did_invoke();

                    // after ccp_invoke, qdisc might have changed epoch_length
                    // due to new rate being set.
                    // accordingly update the measurement epoch window
                    let epoch_length = {
                        self.qdisc.borrow().get_curr_epoch_length()
                    };

                    let rtt_sec = self.flow_state.rtt_estimate as f64 / 1e9;
                    let inflight_bdp = self.flow_state.send_rate * rtt_sec / 1500.0;
                    let inflight_bdp_rounded = round_down_power_of_2(inflight_bdp as u32);

                    let window = inflight_bdp_rounded / epoch_length;
                    self.flow_state.epoch_history.window = std::cmp::max(1, window as usize);
                }
            }
        };

        Ok(minion::LoopState::Continue)
    }
}

fn round_down_power_of_2(x: u32) -> u32 {
    let y = x.leading_zeros();
    if y >= 32 {
        0
    } else {
        1 << (32 - y - 1)
    }
}
