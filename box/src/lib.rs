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
extern crate minion;
extern crate portus;
extern crate slog;

use crossbeam::select;
use minion::Cancellable;
use portus::Result;
use slog::{info, warn};
use std::os::unix::net::UnixDatagram;
use std::rc::Rc;

pub mod serialize;
use self::serialize::{OutBoxFeedbackMsg, QDiscFeedbackMsg};
pub mod udp;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(unused)]
mod ccp;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(unused)]
mod nl;

pub mod qdisc;
use self::qdisc::*;

mod marks;
use self::marks::MarkHistory;

mod readers;
use self::readers::{NlMsgReader, UdpMsgReader, UnixMsgReader};

extern "C" fn bundler_set_cwnd(
    _dp: *mut ccp::ccp_datapath,
    _conn: *mut ccp::ccp_connection,
    _cwnd: u32,
) {
    // no-op
    // TODO: support enforcing a cwnd
}

extern "C" fn bundler_set_rate_abs(
    dp: *mut ccp::ccp_datapath,
    _conn: *mut ccp::ccp_connection,
    rate: u32,
) {
    let dp: *mut DatapathImpl = unsafe { std::mem::transmute((*dp).impl_) };
    // TODO set burst dynamically
    println!("rate: {}", rate);
    unsafe { (*dp).qdisc.set_rate(rate, 100_000).unwrap() };
}

extern "C" fn bundler_set_rate_rel(
    _dp: *mut ccp::ccp_datapath,
    _conn: *mut ccp::ccp_connection,
    _rate: u32,
) {
    // no-op
    // TODO: support enforcing a relative rate
    // can probably deprecate this in libccp
    unimplemented!();
}

extern "C" fn bundler_send_msg(
    dp: *mut ccp::ccp_datapath,
    _conn: *mut ccp::ccp_connection,
    msg: *mut ::std::os::raw::c_char,
    msg_size: ::std::os::raw::c_int,
) -> std::os::raw::c_int {
    // construct the slice
    use std::slice;
    let buf = unsafe { slice::from_raw_parts(msg as *mut u8, msg_size as usize) };

    let dp: *mut DatapathImpl = unsafe { std::mem::transmute((*dp).impl_) };
    unsafe {
        match (*dp).sk.send_to(buf, "/tmp/ccp/0/in") {
            Err(ref e)
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if (*dp).connected {
                    eprintln!("warn: unix socket does not exist...");
                }
                (*dp).connected = false;
                Ok(())
            }
            Err(e) => Err(e),
            Ok(_) => {
                if !(*dp).connected {
                    eprintln!("info: unix socket connected!");
                }
                (*dp).connected = true;
                Ok(())
            }
        }
        .unwrap();
    };
    return 0;
}

extern "C" fn bundler_now() -> u64 {
    time::precise_time_ns()
}

extern "C" fn bundler_since_usecs(then: u64) -> u64 {
    time::precise_time_ns() - then
}

extern "C" fn bundler_after_usecs(usecs: u64) -> u64 {
    time::precise_time_ns() + usecs * 1_000
}

struct DatapathImpl {
    qdisc: Rc<Qdisc>, // qdisc handle
    sk: UnixDatagram,
    connected: bool,
}

#[derive(Default)]
struct BundleFlowState {
    conn: Option<*mut ccp::ccp_connection>,
    marked_packets: MarkHistory,
    prev_send_time: u64,
    prev_send_byte_clock: u64,
    prev_recv_time: u64,
    prev_recv_byte_clock: u64,

    send_rate: f64,
    recv_rate: f64,
    rtt_estimate: u64,

    bdp_estimate_packets: u32,

    lost_bytes: u32,
}

impl BundleFlowState {
    //
    // s1  |\   A       |
    //     | -------    |
    //     |    B   \   |
    // s2  |\-----   ---| r1
    //     |      \ /   |
    //     | -------    |
    // s1' |/   A'  \---| r2
    //     |        /   |
    //     | -------    |
    // s2' |/   B'      |
    //
    //
    // RTT = s2' - s2 = NOW - s2
    // send epoch = s1 -> s2
    // recv epoch = r1 -> r2
    // r1 available with s1'
    // r2 available with s2'
    //
    // We are currently at s2'.
    fn update_measurements(
        &mut self,
        now: u64,
        sent_mark: marks::MarkedInstant,
        recv_mark: OutBoxFeedbackMsg,
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

        let send_epoch_seconds = (s2 - s1) as f64 / 1e9;
        let recv_epoch_seconds = (r2 - r1) as f64 / 1e9;

        let send_epoch_bytes = (s2_bytes - s1_bytes) as f64;
        let recv_epoch_bytes = (r2_bytes - r1_bytes) as f64;

        let send_rate = send_epoch_bytes / send_epoch_seconds;
        let recv_rate = recv_epoch_bytes / recv_epoch_seconds;
        self.send_rate = send_rate;
        self.recv_rate = recv_rate;

        let rtt_s = self.rtt_estimate as f64 / 1e9;
        let bdp_estimate_bytes = send_rate as f64 * rtt_s;
        self.bdp_estimate_packets = (bdp_estimate_bytes / 1514.0) as u32;

        let delta = send_epoch_bytes - recv_epoch_bytes;
        self.lost_bytes = if delta > 0.0 { delta as u32 } else { 0 };

        // s2 now becomes s1 and r2 becomes r1
        self.prev_send_time = s2;
        self.prev_send_byte_clock = s2_bytes;
        self.prev_recv_time = r2;
        self.prev_recv_byte_clock = r2_bytes;
    }
}

pub struct Runtime {
    log: slog::Logger,
    qdisc_handle: Rc<Qdisc>,
    qdisc_recv: crossbeam::Receiver<QDiscFeedbackMsg>,
    outbox_recv: crossbeam::Receiver<OutBoxFeedbackMsg>,
    /// flow measurements
    flow_state: BundleFlowState,
}

impl Runtime {
    pub fn new(
        listen_port: u16,
        iface: String,
        handle: (u32, u32),
        sample_freq: u32,
    ) -> Option<Self> {
        use portus::ipc;
        use portus::ipc::netlink;

        let log = portus::algs::make_logger();

        let nlsk = netlink::Socket::<ipc::Blocking>::new().unwrap();
        let (qdisc_reader, qdisc_recv) = NlMsgReader::make(nlsk);
        let _qdisc_recv_handle = qdisc_reader.spawn();

        let udpsk = udp::Socket::new(listen_port).unwrap();
        let (outbox_reader, outbox_recv) = UdpMsgReader::make(udpsk);
        let _outbox_recv_handle = outbox_reader.spawn();

        let (portus_reader, alg_ready) = UnixMsgReader::make(log.clone());
        let _portus_reader_handle = portus_reader.spawn();

        let qdisc = Qdisc::get(iface, handle);
        qdisc.set_epoch_length(sample_freq).unwrap_or_else(|_| ());
        let qdisc = Rc::new(qdisc);

        // unix socket for sending *to* portus
        let portus_sk = UnixDatagram::unbound().unwrap();

        let dpi = DatapathImpl {
            sk: portus_sk,
            qdisc: qdisc.clone(),
            connected: true,
        };

        let dpi = Box::new(dpi);

        let mut dp = ccp::ccp_datapath {
            set_cwnd: Some(bundler_set_cwnd),
            set_rate_abs: Some(bundler_set_rate_abs),
            set_rate_rel: Some(bundler_set_rate_rel),
            time_zero: time::precise_time_ns(),
            now: Some(bundler_now),
            since_usecs: Some(bundler_since_usecs),
            after_usecs: Some(bundler_after_usecs),
            send_msg: Some(bundler_send_msg),
            impl_: Box::into_raw(dpi) as *mut std::os::raw::c_void,
        };

        let ok = unsafe { ccp::ccp_init(&mut dp) };
        if ok < 0 {
            return None;
        }

        // Wait for algorithm to finish installing datapath programs
        info!(log, "Wait for CCP to install datapath program");
        alg_ready.recv().unwrap();

        info!(log, "Initialize bundle flow in libccp");
        // TODO this is a hack, we are pretending there is only one bundle/flow
        let mut dp_info = ccp::ccp_datapath_info {
            init_cwnd: 10,
            mss: 1514,
            src_ip: 0,
            src_port: 42,
            dst_ip: 0,
            dst_port: 0,
            congAlg: [0i8; 64],
        };

        let conn = unsafe {
            ccp::ccp_connection_start(std::ptr::null_mut::<std::os::raw::c_void>(), &mut dp_info)
        };

        let mut fs: BundleFlowState = Default::default();
        fs.conn = Some(conn);

        info!(log, "Inbox ready");
        Some(Runtime {
            log,
            qdisc_handle: qdisc,
            qdisc_recv,
            outbox_recv,
            flow_state: fs,
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
                    self.flow_state.marked_packets.insert(msg.marked_packet_hash, msg.epoch_time, msg.epoch_bytes);
                }
            },
            recv(self.outbox_recv) -> msg => {
                if let Ok(msg) = msg {
                    // check packet marking
                    let now = time::precise_time_ns();
                    if let Some(mi) = self.flow_state.marked_packets.get(now, msg.marked_packet_hash) {
                        self.flow_state.update_measurements(now, mi, msg);

                        let conn = self.flow_state.conn.unwrap();
                        // set primitives
                        unsafe {
                            (*conn).prims.rtt_sample_us = self.flow_state.rtt_estimate / 1_000;
                            (*conn).prims.rate_outgoing = self.flow_state.send_rate as u64;
                            (*conn).prims.rate_incoming = self.flow_state.recv_rate as u64;
                            (*conn).prims.lost_pkts_sample = self.flow_state.lost_bytes / 1514;
                        }

                        info!(self.log, "CCP Invoke";
                              "rtt" => unsafe { (*conn).prims.rtt_sample_us },
                              "rate_outgoing" => unsafe { (*conn).prims.rate_outgoing },
                              "rate_incoming" => unsafe { (*conn).prims.rate_incoming },
                              "lost_pkts_sample" => unsafe { (*conn).prims.lost_pkts_sample },
                        );

                        // ccp_invoke
                        let ok = unsafe { ccp::ccp_invoke(conn) };
                        if ok < 0 {
                            warn!(self.log, "CCP Invoke Error"; "code" => ok);
                        }
                    }
                }
            },
        };

        Ok(minion::LoopState::Continue)
    }
}
