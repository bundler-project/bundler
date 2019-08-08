use bundler::{IP_HEADER_LENGTH, IP_PROTO_TCP, MAC_HEADER_LENGTH, PROTO_IN_IP_HEADER};
use minion::Cancellable;
use slog::{info, o};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use structopt::StructOpt;

#[derive(Clone, Debug, StructOpt)]
#[structopt(name = "playback", about = "playback tcpdump traces for bundler")]
struct Opt {
    #[structopt(short = "i", long = "inbox")]
    inbox_dump_file: std::path::PathBuf,
    #[structopt(short = "o", long = "outbox")]
    outbox_dump_file: std::path::PathBuf,
    #[structopt(long = "with_ethernet")]
    with_ethernet: bool,
}

fn main() {
    let opt = Opt::from_args();
    let log = portus::algs::make_logger();
    let root_log = log.new(o!("node" => "script"));

    let inbox_capture = pcap::Capture::from_file(opt.inbox_dump_file.clone()).unwrap();
    let outbox_capture = pcap::Capture::from_file(opt.outbox_dump_file.clone()).unwrap();

    let (outbox_report_tx, epoch_length_adjust_rx) = glue();
    let (qdisc_ctl_tx, qdisc_ctl_rx) = mpsc::channel();

    let (inbox_player, qdisc_match_rx) = InboxCapturePlayer::new(
        log.new(o!("node" => "inbox_player")),
        inbox_capture,
        qdisc_ctl_rx,
        opt.with_ethernet,
    );

    info!(root_log, "starting inbox playback");
    let inbox_player_runner = inbox_player.spawn();

    info!(root_log, "starting outbox");
    let outbox_feedback_rx = start_outbox(
        log.new(o!("node" => "outbox")),
        opt.clone(),
        outbox_capture,
        epoch_length_adjust_rx,
    );

    info!(root_log, "starting inbox runtime");

    // Inbox runtime contains an Rc which is not Send, so use this
    // signal to know when it's done constructing.
    let (s, r) = mpsc::channel::<()>();
    let log1 = log.new(o!("node" => "inbox_runtime"));
    std::thread::spawn(move || {
        let mut rt = new_inbox_runtime(
            log1,
            qdisc_match_rx,
            outbox_feedback_rx,
            outbox_report_tx,
            qdisc_ctl_tx,
        )
        .unwrap();
        s.send(()).unwrap();
        rt.run().unwrap();
    });

    r.recv().unwrap();
    inbox_player_runner.wait().unwrap();
    info!(root_log, "done");

    // sleep for log to flush
    std::thread::sleep(std::time::Duration::from_millis(100));
}

struct InboxCapturePlayer {
    log: slog::Logger,
    cap: pcap::Capture<pcap::Offline>,
    qdisc_match_tx: crossbeam::Sender<bundler::serialize::QDiscFeedbackMsg>,
    qdisc_ctl_rx: mpsc::Receiver<u32>,
    bytes_recv: u64,
    epoch_sample_rate: u32,
    with_ethernet: bool,
    ip_header_start: usize,
    tcp_header_start: usize,
}

impl InboxCapturePlayer {
    fn new(
        log: slog::Logger,
        cap: pcap::Capture<pcap::Offline>,
        qdisc_ctl_rx: mpsc::Receiver<u32>,
        with_ethernet: bool,
    ) -> (
        Self,
        crossbeam::Receiver<bundler::serialize::QDiscFeedbackMsg>,
    ) {
        let ip_header_start = if !with_ethernet { 0 } else { MAC_HEADER_LENGTH };
        let tcp_header_start = ip_header_start + IP_HEADER_LENGTH;
        let (tx, rx) = crossbeam::bounded(0);
        (
            InboxCapturePlayer {
                log,
                cap,
                qdisc_match_tx: tx,
                qdisc_ctl_rx,
                bytes_recv: 0,
                epoch_sample_rate: 128,
                with_ethernet,
                ip_header_start,
                tcp_header_start,
            },
            rx,
        )
    }
}

impl minion::Cancellable for InboxCapturePlayer {
    type Error = ();
    fn for_each(&mut self) -> Result<minion::LoopState, Self::Error> {
        match self.qdisc_ctl_rx.try_recv() {
            Ok(new_epoch_length_packets) => {
                if new_epoch_length_packets > 0 {
                    self.epoch_sample_rate = new_epoch_length_packets;
                }
            }
            Err(mpsc::TryRecvError::Empty) => (),
            Err(mpsc::TryRecvError::Disconnected) => unreachable!(),
        }
        match self.cap.next() {
            Ok(pkt) => {
                let now = pkt.header.ts;
                let now = now.tv_sec as u64 * 1_000_000_000 + now.tv_usec as u64 * 1_000; // ns since epoch
                let data = pkt.data;

                // Is this a TCP packet?
                if data[self.ip_header_start + PROTO_IN_IP_HEADER] != IP_PROTO_TCP {
                    return Ok(minion::LoopState::Continue);
                }

                self.bytes_recv += pkt.header.len as u64;
                if !self.with_ethernet {
                    self.bytes_recv += MAC_HEADER_LENGTH as u64;
                }

                let hash =
                    bundler::hash::hash_packet(self.ip_header_start, self.tcp_header_start, data);
                if hash % self.epoch_sample_rate == 0 {
                    let msg = bundler::serialize::QDiscFeedbackMsg {
                        bundle_id: 42,
                        marked_packet_hash: hash,
                        curr_qlen: 100,
                        epoch_bytes: self.bytes_recv,
                        epoch_time: now,
                    };

                    self.qdisc_match_tx.send(msg).unwrap();
                }

                Ok(minion::LoopState::Continue)
            }
            _ => {
                info!(self.log, "inbox playback done");
                Ok(minion::LoopState::Break)
            }
        }
    }
}

/// Communication between inbox <--> outbox
fn glue() -> (
    mpsc::Sender<bundler::serialize::OutBoxReportMsg>,
    mpsc::Receiver<u32>,
) {
    let (outbox_report_tx, outbox_report_rx): (
        mpsc::Sender<bundler::serialize::OutBoxReportMsg>,
        mpsc::Receiver<bundler::serialize::OutBoxReportMsg>,
    ) = mpsc::channel();
    let (epoch_length_adjust_tx, epoch_length_adjust_rx): (mpsc::Sender<u32>, mpsc::Receiver<u32>) =
        mpsc::channel();

    std::thread::spawn(move || loop {
        match outbox_report_rx.recv() {
            Ok(msg) => {
                epoch_length_adjust_tx
                    .send(msg.epoch_length_packets)
                    .unwrap();
            }
            Err(_) => {
                return;
            }
        }
    });

    (outbox_report_tx, epoch_length_adjust_rx)
}

fn start_outbox<T: pcap::Activated + Send + 'static>(
    log: slog::Logger,
    outbox_opt: Opt,
    outbox_capture: pcap::Capture<T>,
    epoch_length_adjust_rx: mpsc::Receiver<u32>,
) -> crossbeam::Receiver<bundler::serialize::OutBoxFeedbackMsg> {
    // outbox sends on tx when it sees an epoch boundary packet
    let (epoch_boundary_tx, epoch_boundary_rx) = crossbeam::bounded::<(u64, u32, u64)>(0);
    let (outbox_feedback_tx, outbox_feedback_rx) = crossbeam::bounded(0);

    std::thread::spawn(move || loop {
        let (ts, hash, recvd) = match epoch_boundary_rx.recv() {
            Ok((ts, hash, recvd)) => (ts, hash, recvd),
            Err(_) => break,
        };

        let msg = bundler::serialize::OutBoxFeedbackMsg {
            bundle_id: 42,
            marked_packet_hash: hash,
            epoch_bytes: recvd,
            epoch_time: ts,
        };

        outbox_feedback_tx.send(msg).unwrap();
    });

    std::thread::spawn(move || {
        info!(log.clone(), "starting outbox playback");
        bundler::outbox::start_outbox(
            outbox_capture,
            epoch_boundary_tx,
            epoch_length_adjust_rx,
            128,
            !outbox_opt.with_ethernet,
            log.clone(),
        )
        .unwrap_err();
        info!(log.clone(), "outbox done");
    });

    outbox_feedback_rx
}

fn new_inbox_runtime(
    log: slog::Logger,
    qdisc_recv: crossbeam::Receiver<bundler::serialize::QDiscFeedbackMsg>,
    outbox_recv: crossbeam::Receiver<bundler::serialize::OutBoxFeedbackMsg>,
    outbox_report: mpsc::Sender<bundler::serialize::OutBoxReportMsg>,
    qdisc_ctl: mpsc::Sender<u32>,
) -> Option<bundler::inbox::Runtime<FakeInboxQdisc>> {
    let qdisc: FakeInboxQdisc = FakeInboxQdisc {
        cwnd_bytes: 0,
        rate_bytes_per_sec: 0,
        observed_sending_bytes_per_sec: 0,
        rtt_ns: 0,
        min_rtt_ns: 0,
        curr_epoch_length: 0,
        outbox_report,
        qdisc_ctl,
    };
    let qdisc = Rc::new(RefCell::new(qdisc));
    bundler::inbox::Runtime::with_qdisc(qdisc, qdisc_recv, outbox_recv, log)
}

/// Does nothing - the actual "qdisc" functionality is based on the pcap trace
struct FakeInboxQdisc {
    cwnd_bytes: u32,
    rate_bytes_per_sec: u32,
    observed_sending_bytes_per_sec: u64,
    rtt_ns: u64,
    min_rtt_ns: u64,
    curr_epoch_length: u32,
    outbox_report: mpsc::Sender<bundler::serialize::OutBoxReportMsg>,
    qdisc_ctl: mpsc::Sender<u32>,
}

impl bundler::inbox::datapath::Datapath for FakeInboxQdisc {
    fn set_approx_cwnd(&mut self, cwnd_bytes: u32) -> Result<(), ()> {
        self.cwnd_bytes = cwnd_bytes;
        Ok(())
    }

    fn set_rate(&mut self, rate: u32) -> Result<(), ()> {
        self.rate_bytes_per_sec = rate;
        Ok(())
    }

    fn update_rtt(&mut self, rtt_ns: u64) -> Result<(), ()> {
        self.rtt_ns = rtt_ns;
        self.min_rtt_ns = std::cmp::min(self.min_rtt_ns, rtt_ns);
        Ok(())
    }

    fn set_epoch_length(&mut self, epoch_length_packets: u32) -> Result<(), portus::Error> {
        if self.curr_epoch_length > 0 {
            return Ok(());
        }

        if self.curr_epoch_length == epoch_length_packets {
            return Ok(());
        }

        self.curr_epoch_length = epoch_length_packets;
        // tell the outbox what the epoch length is
        let msg = bundler::serialize::OutBoxReportMsg {
            bundle_id: 42,
            epoch_length_packets,
        };

        self.outbox_report.send(msg).unwrap_or_else(|_| ());
        self.qdisc_ctl
            .send(epoch_length_packets)
            .unwrap_or_else(|_| ());
        Ok(())
    }

    fn update_send_rate(&mut self, observed_sending_bytes_per_sec: u64) {
        self.observed_sending_bytes_per_sec = observed_sending_bytes_per_sec;
        let epoch_length = bundler::inbox::datapath::get_epoch_length(
            self.observed_sending_bytes_per_sec as f64,
            self.min_rtt_ns as f64 / 1e9,
        );
        self.set_epoch_length(epoch_length).unwrap_or_else(|_| ());
    }

    fn get_curr_epoch_length(&self) -> u32 {
        self.curr_epoch_length
    }
}
