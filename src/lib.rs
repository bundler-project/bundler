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

use crossbeam::select;
use minion::Cancellable;
use portus::ipc;
use portus::ipc::netlink;
use portus::ipc::Ipc;
use portus::Result;
use std::os::unix::net::UnixDatagram;

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
        let s = NlMsgReader(nl, vec![0u8; 20], send);
        (s, recv)
    }
}

impl Cancellable for NlMsgReader {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        self.0.recv(&mut self.1[0..20])?;
        let m = QDiscFeedbackMsg::from_slice(&self.1[0..20]);
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
        self.0.recv(&mut self.1[0..28])?;
        let m = OutBoxFeedbackMsg::from_slice(&self.1[0..28]);
        self.2.send(m)?;
        Ok(minion::LoopState::Continue)
    }
}

struct UnixMsgReader(UnixDatagram, Vec<u8>);

impl UnixMsgReader {
    fn make() -> Self {
        let unix = UnixDatagram::bind("/tmp/ccp/0/out").unwrap();
        UnixMsgReader(unix, vec![0u8; 1024])
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
            println!("error in ccp_read_msg");
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
    unsafe { (*dp).sk.send_to(buf, "/tmp/ccp/0/in").unwrap() };
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
}

#[derive(Default)]
struct BundleFlowState {
    conn: Option<*mut ccp::ccp_connection>,
    marked_packets: fnv::FnvHashMap<u32, u64>,
    curr_epoch: u64,
    prev_recv_epoch: u64,
    send_rate: f64,
    recv_rate: f64,
    rtt_estimate: u64,
}

impl Runtime {
    pub fn new() -> Option<Self> {
        let nlsk = netlink::Socket::<ipc::Blocking>::new().unwrap();
        let (qdisc_reader, qdisc_recv) = NlMsgReader::make(nlsk);
        let qdisc_recv_handle = qdisc_reader.spawn();

        let udpsk = udp::Socket::new(28316).unwrap();
        let (outbox_reader, outbox_recv) = UdpMsgReader::make(udpsk);
        let outbox_recv_handle = outbox_reader.spawn();

        let portus_reader = UnixMsgReader::make();
        let portus_reader_handle = portus_reader.spawn();

        // TODO For now assumes root qdisc on the 10gp1 interface, but this
        // should be configurable  or we should add a deterministic way to
        // discover it correctly.
        let qdisc = Qdisc::get(String::from("10gp1"), (1, 0));

        // unix socket for sending *to* portus
        let portus_sk = UnixDatagram::unbound().unwrap();

        let dpi = DatapathImpl {
            sk: portus_sk,
            qdisc,
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

        // this is a hack, we are pretending there is only one bundle/flow
        let mut dp_info = ccp::ccp_datapath_info {
            init_cwnd: 10,
            mss: 1500,
            src_ip: 0,
            src_port: 0,
            dst_ip: 0,
            dst_port: 0,
            congAlg: [0i8; 64],
        };

        let conn = unsafe {
            ccp::ccp_connection_start(std::ptr::null_mut::<std::os::raw::c_void>(), &mut dp_info)
        };

        let mut fs: BundleFlowState = Default::default();
        fs.conn = Some(conn);

        Some(Runtime {
            qdisc_recv,
            qdisc_recv_handle,
            outbox_recv,
            outbox_recv_handle,
            portus_reader_handle,
            flow_state: fs,
        })
    }
}

impl Cancellable for Runtime {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        select!{
            recv(self.qdisc_recv) -> msg => {
                if let Ok(msg) = msg {
                    // remember the marked packet's send time
                    // so we can get its RTT later
                    self.flow_state.marked_packets.insert(msg.marked_packet_hash, msg.epoch_time);
                    // update r_in
                    let elapsed = (msg.epoch_time - self.flow_state.curr_epoch) as f64 / 1e9;
                    self.flow_state.send_rate = msg.epoch_bytes as f64 / elapsed;

                    // update the current epoch time
                    self.flow_state.curr_epoch = msg.epoch_time;
                }
            },
            recv(self.outbox_recv) -> msg => {
                if let Ok(msg) = msg {
                    // check packet marking
                    if let Some(pkt_timestamp) = self.flow_state.marked_packets.get(&msg.marked_packet_hash) {
                        self.flow_state.rtt_estimate = time::precise_time_ns() - pkt_timestamp;
                        let epoch_elapsed = (msg.recv_time - self.flow_state.prev_recv_epoch) as f64 / 1e9;
                        self.flow_state.recv_rate = msg.epoch_bytes as f64 / epoch_elapsed;

                        self.flow_state.prev_recv_epoch = msg.recv_time;
                    }

                    let conn = self.flow_state.conn.unwrap();
                    // set primitives
                    unsafe {
                        (*conn).prims.rtt_sample_us = self.flow_state.rtt_estimate / 1_000;
                        (*conn).prims.rate_outgoing = self.flow_state.send_rate as u64;
                        (*conn).prims.rate_incoming = self.flow_state.recv_rate as u64;
                    }

                    // ccp_invoke
                    let ok = unsafe { ccp::ccp_invoke(conn) };
                    if ok < 0 {
                        println!("ccp invoke error: {:?}", ok);
                    }
                }
            },
        };

        Ok(minion::LoopState::Continue)
    }
}
