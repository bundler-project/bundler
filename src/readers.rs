use crate::serialize::{OutBoxFeedbackMsg, QDiscFeedbackMsg};
use crate::udp;
use minion::Cancellable;
use portus::ipc;
use portus::ipc::netlink;
use portus::ipc::Ipc;
use slog::warn;
use std::os::unix::net::UnixDatagram;

pub struct NlMsgReader(
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

pub struct UdpMsgReader(udp::Socket, Vec<u8>, crossbeam::Sender<OutBoxFeedbackMsg>);

impl UdpMsgReader {
    pub fn make(udp: udp::Socket) -> (Self, crossbeam::Receiver<OutBoxFeedbackMsg>) {
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

use std::sync::Arc;

pub struct UnixMsgReader(
    UnixDatagram,
    Vec<u8>,
    crossbeam::Sender<()>,
    u32,
    slog::Logger,
    Arc<libccp::Datapath>,
);

impl UnixMsgReader {
    pub fn make(
        logger: slog::Logger,
        dp: Arc<libccp::Datapath>,
    ) -> (Self, crossbeam::Receiver<()>) {
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

        let s = UnixMsgReader(sock, vec![0u8; 1024], send, 0, logger, dp);
        (s, recv)
    }
}

impl Cancellable for UnixMsgReader {
    type Error = portus::Error;

    fn for_each(&mut self) -> std::result::Result<minion::LoopState, Self::Error> {
        let bytes_read = self.0.recv(&mut self.1[..])?;

        self.5
            .recv_msg(&mut self.1[..bytes_read])
            .map_err(|e| {
                warn!(self.4, "ccp_read_msg error"; "code" => ?e);
            })
            .unwrap_or_else(|_| ());

        self.3 += 1;
        if self.3 == 1 {
            self.2.send(())?;
        }

        Ok(minion::LoopState::Continue)
    }
}
