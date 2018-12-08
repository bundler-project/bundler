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
use portus::ipc;
use portus::ipc::netlink;
use portus::ipc::Ipc;
use portus::Result;
use slog::{info, warn};
use std::os::unix::net::UnixDatagram;
use std::collections::VecDeque;

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

struct NlMsgReader(
    netlink::Socket<ipc::Blocking>,
    Vec<u8>,
    crossbeam::Sender<QDiscFeedbackMsg>,
);

impl NlMsgReader {
    pub fn make(
        nl: netlink::Socket<ipc::Blocking>,
    ) -> (Self, crossbeam::Receiver<QDiscFeedbackMsg>) {
        let (send, recv) = crossbeam::unbounded();
        let s = NlMsgReader(nl, vec![0u8; 100], send);
        (s, recv)
    }
}

impl Cancellable for NlMsgReader {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        self.0.recv(&mut self.1[0..100])?;
        let m = QDiscFeedbackMsg::from_slice(&self.1[0..24]);
        self.2.send(m)?;
        Ok(minion::LoopState::Continue)
    }
}

struct UdpMsgReader(udp::Socket, Vec<u8>, crossbeam::Sender<OutBoxFeedbackMsg>);

impl UdpMsgReader {
    fn make(udp: udp::Socket) -> (Self, crossbeam::Receiver<OutBoxFeedbackMsg>) {
        let (send, recv) = crossbeam::unbounded();
        let s = UdpMsgReader(udp, vec![0u8; 28], send);
        (s, recv)
    }
}

impl Cancellable for UdpMsgReader {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        self.0.recv(&mut self.1[0..24])?;
        let m = OutBoxFeedbackMsg::from_slice(&self.1[0..24]);
        self.2.send(m)?;
        Ok(minion::LoopState::Continue)
    }
}

struct UnixMsgReader(UnixDatagram, Vec<u8>, crossbeam::Sender<()>, u32, slog::Logger);

impl UnixMsgReader {
    fn make(logger: slog::Logger) -> (Self, crossbeam::Receiver<()>) {
        let addr = "/tmp/ccp/0/out";

        match std::fs::create_dir_all("/tmp/ccp/0").err() {
            Some(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Some(e) => Err(e),
            None => Ok(()),
        }
        .unwrap();

        match std::fs::remove_file(&addr).err() {
            Some(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Some(e) => Err(e),
            None => Ok(()),
        }
        .unwrap();

        let sock = UnixDatagram::bind(addr).unwrap();

        let (send, recv) = crossbeam::bounded(0);

        let s = UnixMsgReader(sock, vec![0u8; 1024], send, 0, logger);
        (s, recv)
    }
}

impl Cancellable for UnixMsgReader {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        let bytes_read = self.0.recv(&mut self.1[..])?;

        // cast the vec in self to a *mut c_char
        let buf = self.1.as_mut_ptr() as *mut ::std::os::raw::c_char;
        let ok = unsafe { ccp::ccp_read_msg(buf, bytes_read as i32) };
        if ok < 0 {
            warn!(self.4, "ccp_read_msg error"; "code" => ok);
        }

        self.3 += 1;
        if self.3 == 1 {
            self.2.send(())?;
        }

        Ok(minion::LoopState::Continue)
    }
}

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

pub struct Runtime {
    log: slog::Logger,
    qdisc_recv: crossbeam::Receiver<QDiscFeedbackMsg>,
    qdisc_recv_handle: minion::Handle<portus::Error>,
    outbox_recv: crossbeam::Receiver<OutBoxFeedbackMsg>,
    outbox_recv_handle: minion::Handle<portus::Error>,
    portus_reader_handle: minion::Handle<portus::Error>,
    flow_state: BundleFlowState,
}

struct DatapathImpl {
    qdisc: Qdisc, // qdisc handle
    sk: UnixDatagram,
    connected: bool,
}

#[derive(Clone, Copy)]
struct MarkedInstant {
    time: u64,
    pkt_hash: u32,
    send_byte_clock: u64,
}

#[derive(Default)]
pub struct MarkHistory {
    marks: VecDeque<MarkedInstant>,
}

impl MarkHistory {
    fn insert(&mut self, pkt_hash: u32, time: u64, send_byte_clock: u64) {
        self.marks.push_back(MarkedInstant {
            time,
            pkt_hash,
            send_byte_clock,
        })
    }

    // TODO can implement binary search for perf
    fn find_idx(&self, pkt_hash: u32) -> Option<usize> {
        for i in 0..self.marks.len() {
            if self.marks[i].pkt_hash == pkt_hash {
                return Some(i);
            }
        }

        None
    }

    fn get(&mut self, _now: u64, pkt_hash: u32) -> Option<MarkedInstant> {
        let idx = self.find_idx(pkt_hash)?;
        self.marks.drain(0..(idx + 1)).last()
    }
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
}

impl Runtime {
    pub fn new(listen_port: u16, iface: String, handle: (u32, u32)) -> Option<Self> {
        let log = portus::algs::make_logger();

        let nlsk = netlink::Socket::<ipc::Blocking>::new().unwrap();
        let (qdisc_reader, qdisc_recv) = NlMsgReader::make(nlsk);
        let qdisc_recv_handle = qdisc_reader.spawn();

        let udpsk = udp::Socket::new(listen_port).unwrap();
        let (outbox_reader, outbox_recv) = UdpMsgReader::make(udpsk);
        let outbox_recv_handle = outbox_reader.spawn();

        let (portus_reader, alg_ready) = UnixMsgReader::make(log.clone());
        let portus_reader_handle = portus_reader.spawn();

        // TODO For now assumes root qdisc on the 10gp1 interface, but this
        // should be configurable  or we should add a deterministic way to
        // discover it correctly.
        let qdisc = Qdisc::get(iface, handle);

        // unix socket for sending *to* portus
        let portus_sk = UnixDatagram::unbound().unwrap();

        let dpi = DatapathImpl {
            sk: portus_sk,
            qdisc,
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
        // this is a hack, we are pretending there is only one bundle/flow
        let mut dp_info = ccp::ccp_datapath_info {
            init_cwnd: 10,
            mss: 1500,
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
            qdisc_recv,
            qdisc_recv_handle,
            outbox_recv,
            outbox_recv_handle,
            portus_reader_handle,
            flow_state: fs,
        })
    }

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
    fn update_measurements(&mut self, now: u64, sent_mark: MarkedInstant, recv_mark: OutBoxFeedbackMsg) {
        let s1 = self.flow_state.prev_send_time;
        let s1_bytes = self.flow_state.prev_send_byte_clock;
        let s2 = sent_mark.time;
        let s2_bytes = sent_mark.send_byte_clock;
        let r1 = self.flow_state.prev_recv_time;
        let r1_bytes = self.flow_state.prev_recv_byte_clock;
        let r2 = recv_mark.epoch_time;
        let r2_bytes = recv_mark.epoch_bytes;

        // rtt is current time - sent mark time
        self.flow_state.rtt_estimate = now - s2;

        let send_epoch_seconds = (s2 - s1) as f64 / 1e9;
        let recv_epoch_seconds = (r2 - r1) as f64 / 1e9;

        let send_epoch_bytes = (s2_bytes - s1_bytes) as f64;
        let recv_epoch_bytes = (r2_bytes - r1_bytes) as f64;

        let send_rate = send_epoch_bytes / send_epoch_seconds;
        let recv_rate = recv_epoch_bytes / recv_epoch_seconds;
        self.flow_state.send_rate = send_rate;
        self.flow_state.recv_rate = recv_rate;

        // s2 now becomes s1 and r2 becomes r1
        self.flow_state.prev_send_time = s2;
        self.flow_state.prev_send_byte_clock = s2_bytes;
        self.flow_state.prev_recv_time = r2;
        self.flow_state.prev_recv_byte_clock = r2_bytes;
    }
}

impl Cancellable for Runtime {
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
                        self.update_measurements(now, mi, msg);
                        
                        let conn = self.flow_state.conn.unwrap();
                        // set primitives
                        unsafe {
                            (*conn).prims.rtt_sample_us = self.flow_state.rtt_estimate / 1_000;
                            (*conn).prims.rate_outgoing = self.flow_state.send_rate as u64;
                            (*conn).prims.rate_incoming = self.flow_state.recv_rate as u64;
                        }

                        info!(self.log, "CCP Invoke";
                              "rtt" => unsafe { (*conn).prims.rtt_sample_us },
                              "rate_outgoing" => unsafe { (*conn).prims.rate_outgoing },
                              "rate_incoming" => unsafe { (*conn).prims.rate_incoming },
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
