extern crate bundler;
extern crate bytes;
extern crate clap;
extern crate time;

#[cfg(target_os = "linux")]
fn main() {
    use clap::{value_t, App, Arg};
    use pcap::{Capture, Device};

    use bundler::serialize;
    use bundler::serialize::OutBoxFeedbackMsg;

    use std::net::UdpSocket;
    use std::sync::mpsc;
    use std::thread;
    let matches = App::new("outbox")
        .version("0.1")
        .arg(
            Arg::with_name("iface")
                .short("i")
                .long("iface")
                .help("Interface to listen on")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("filter")
                .short("f")
                .long("filter")
                .help("pcap filter for packets")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("sample_rate")
                .short("s")
                .long("sample_rate")
                .help("sample 1 out of every [sample_rate] packets")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("inbox")
                .long("inbox")
                .help("address of inbox")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("no_ethernet")
                .long("no_ethernet")
                .short("e")
                .help("if true, assumes captured packets do not have ethernet")
                .takes_value(false),
        )
        .get_matches();

    let iface = matches.value_of("iface").unwrap();
    let filter = matches.value_of("filter").unwrap();
    let mut sample_rate = value_t!(matches.value_of("sample_rate"), u32).unwrap();
    let no_ethernet = matches.is_present("no_ethernet");

    let devs = Device::list().unwrap();
    let dev = devs.into_iter().find(|dev| dev.name == iface);
    let mut cap = Capture::from_device(dev.unwrap())
        .unwrap()
        .promisc(false) // Promiscuous mode because the packets are not destined for our IP
        .snaplen(42) // We only need up to byte 42 to read the sequence number
        .immediate_mode(true)
        .open()
        .unwrap();
    cap.filter(filter).unwrap();

    let mut inbox = matches.value_of("inbox").map(|a| {
        use std::net::ToSocketAddrs;
        a.to_socket_addrs().unwrap().next().unwrap()
    });

    let sock = UdpSocket::bind("0.0.0.0:28317").expect("failed to create UDP socket");
    let recv_sock = sock.try_clone().expect("Clone recv_sock");
    if let None = inbox {
        let mut buf = [0u8; 64];
        match recv_sock.recv_from(&mut buf) {
            Ok((bytes, addr)) => {
                inbox = Some(addr);
                if bytes == 8 {
                    let msg = serialize::OutBoxReportMsg::from_slice(&buf);
                    sample_rate = msg.epoch_length_packets;
                }
            }
            Err(e) => println!("{:?}", e),
        }
    }

    let (tx, rx) = crossbeam::unbounded::<(u64, u32, u64)>();
    let log = portus::algs::make_logger();

    let out = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--short")
        .arg("HEAD")
        .output();
    let commit = String::from_utf8(out.unwrap().stdout);
    slog::info!(&log, "bundler commit"; "commit" => ?commit);

    let feedback_log = log.clone();
    let recv_log = log.clone();

    thread::spawn(move || loop {
        let (ts, hash, recvd) = match rx.recv() {
            Ok(x) => x,
            Err(e) => {
                slog::error!(feedback_log, "Error getting next OutBoxFeedbackMsg to send"; "err" => ?e);
                break;
            }
        };
        let msg = OutBoxFeedbackMsg {
            bundle_id: 42,
            marked_packet_hash: hash,
            epoch_bytes: recvd,
            epoch_time: ts,
        };

        sock.send_to(msg.as_bytes().as_slice(), inbox.unwrap())
            .expect("failed to send on UDP socket");
    });

    let (s, r) = mpsc::channel();
    thread::spawn(move || {
        let mut recv_buf = [0u8; 64];
        loop {
            match recv_sock.recv(&mut recv_buf) {
                Ok(bytes) => {
                    if bytes == 8 {
                        let msg = serialize::OutBoxReportMsg::from_slice(&recv_buf);
                        s.send(msg.epoch_length_packets).unwrap();
                    }
                }
                Err(e) => slog::warn!(recv_log, "Error getting OutBoxReportMsg"; "err" => ?e),
            }
        }
    });

    slog::info!(&log, "starting outbox");

    bundler::outbox::start_outbox(cap, tx, r, sample_rate, no_ethernet, log)
        .expect("outbox returned error");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("Runs on Linux only")
}
