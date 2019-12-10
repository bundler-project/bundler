//! This is the sender side. Its responsibilities are to:
//! 1. communicate with the pacing qdisc
//! 2. communicate out-of-band with the receiver side of the virutal congestion tunnel
//! 3. enforce measurements and issue calls to libccp

#[cfg(target_os = "linux")]
use self::datapath::qdisc::*;

use self::datapath::Datapath;
use self::flow_state::BundleFlowState;
use self::readers::UnixMsgReader;
use crate::prio::{FlowInfo, Prioritizer};
use crate::serialize::{OutBoxFeedbackMsg, QDiscPrioMsg, QDiscRecvMsgs};
use crossbeam::select;
use crossbeam::tick;
use minion::Cancellable;
use slog::{debug, info};
use std::cell::RefCell;
use std::os::unix::net::UnixDatagram;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use self::readers::NlMsgReader;

pub mod datapath;
mod flow_state;
#[cfg(target_os = "linux")]
mod nl;
pub mod readers;
pub mod udp;

pub struct DatapathImpl {
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

pub struct ConnectionImpl<Q: Datapath> {
    qdisc: Rc<RefCell<Q>>, // qdisc handle
}

impl<Q: Datapath> libccp::CongestionOps for ConnectionImpl<Q> {
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

pub struct Runtime<Q, P>
where
    Q: Datapath + 'static,
{
    log: slog::Logger,
    qdisc_recv: crossbeam::Receiver<QDiscRecvMsgs>,
    outbox_recv: crossbeam::Receiver<OutBoxFeedbackMsg>,
    // Do not allow a reference to flow_state to escape.
    // Doing so would be unsafe, because the lifetime is
    // declared as 'static when it is actually the same
    // as the lifetime of Arc<libccp::Datapath>.
    flow_state: BundleFlowState<'static, Q>,
    qdisc: Rc<RefCell<Q>>,
    prioritizer: Option<P>,
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
impl<Q: Datapath, P> Drop for Runtime<Q, P> {
    fn drop(&mut self) {}
}

#[cfg(target_os = "linux")]
impl<P> Runtime<Qdisc, P> {
    pub fn new(
        log: slog::Logger,
        prioritizer: Option<P>,
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

        let (outbox_reader, outbox_recv) = self::readers::UdpMsgReader::make(udpsk);
        let _outbox_recv_handle = outbox_reader.spawn();

        let mut qdisc = Qdisc::bind(
            log.clone(),
            iface,
            handle,
            use_dynamic_epoch,
            outbox_found_rx,
            outbox_report,
        )
        .ok()?;

        qdisc.set_epoch_length(sample_freq).unwrap_or_else(|_| ());

        let qdisc = Rc::new(RefCell::new(qdisc));
        Runtime::with_qdisc(qdisc, prioritizer, qdisc_recv, outbox_recv, log)
    }
}

impl<Q: Datapath, P> Runtime<Q, P> {
    pub fn with_qdisc(
        qdisc: Rc<RefCell<Q>>,
        prioritizer: Option<P>,
        qdisc_recv: crossbeam::Receiver<QDiscRecvMsgs>,
        outbox_recv: crossbeam::Receiver<OutBoxFeedbackMsg>,
        log: slog::Logger,
    ) -> Option<Self> {
        // unix socket for sending *to* portus
        let portus_sk = UnixDatagram::unbound().unwrap();

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

        let mut fs: BundleFlowState<Q> = Default::default();
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
            prioritizer,
            invoke_ticker,
            ready_to_invoke: false,
            datapath: dp,
        })
    }
}

impl<Q: Datapath, P: Prioritizer> Runtime<Q, P> {
    fn handle_prio_msg(&mut self, msg: QDiscPrioMsg) {
        if let Some(ref mut p) = self.prioritizer {
            let f = FlowInfo {
                src_ip: msg.src_ip,
                src_port: msg.src_port,
                dst_ip: msg.dst_ip,
                dst_port: msg.dst_port,
            };

            debug!(self.log, "flow prio sample";
                "flow_id" => msg.flow_id,
                "flow_info" => ?f,
            );

            let wt = p.assign_priority(f);
            {
                let mut q = self.qdisc.borrow_mut();
                q.update_flow_prio(msg.flow_id, wt).unwrap_or_else(|_| ());
            }
        }
    }
}

impl<Q: Datapath, P: Prioritizer> minion::Cancellable for Runtime<Q, P> {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        select! {
            recv(self.qdisc_recv) -> msg => {
                match msg {
                    Ok(QDiscRecvMsgs::BundleFeedback(msg)) => {
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
                    Ok(QDiscRecvMsgs::FlowPrio(msg)) => {
                        self.handle_prio_msg(msg);
                    }
                    _ => (),
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

                        info!(self.log, "new measurements";
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
                    let inflight_bdp_rounded = crate::round_down_power_of_2(inflight_bdp as u32);

                    let window = inflight_bdp_rounded / epoch_length;
                    self.flow_state.epoch_history.window = std::cmp::max(1, window as usize);
                }
            }
        };

        Ok(minion::LoopState::Continue)
    }
}
