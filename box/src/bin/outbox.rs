extern crate bundler;
extern crate bytes;
extern crate clap;
extern crate time;

use clap::{value_t, App, Arg};
use pcap::{Capture, Device};

use bundler::serialize;
use bundler::serialize::OutBoxFeedbackMsg;

use slog::{debug, info};
use std::net::UdpSocket;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

// Header lengths
const MAC_HEADER_LENGTH: usize = 14;
const IP_HEADER_LENGTH: usize = 20;
// Locations in headers
const PROTO_IN_IP_HEADER: usize = 9;
// Values
const IP_PROTO_TCP: u8 = 6;

/// TCP Header
///    0                   1                   2                   3
///    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |          Source Port          |       Destination Port        |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                        Sequence Number                        |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                    Acknowledgment Number                      |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |  Data |           |U|A|P|R|S|F|                               |
///   | Offset| Reserved  |R|C|S|S|Y|I|            Window             |
///   |       |           |G|K|H|T|N|N|                               |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |           Checksum            |         Urgent Pointer        |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                    Options                    |    Padding    |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                             data                              |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///
/// IP Header
///
///    0                   1                   2                   3
///    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |Version|  IHL  |Type of Service|          Total Length         |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |         Identification        |Flags|      Fragment Offset    |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |  Time to Live |    Protocol   |         Header Checksum       |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                       Source Address                          |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                    Destination Address                        |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                    Options                    |    Padding    |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///
/// Pseudo-header for packet hashing:
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                       Source Address (ipv4 header)            |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |                    Destination Address (ipv4 header)          |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |       Source Port (tcp hdr)   | Destination Port (tcp hdr)    |
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///   |      Identification (ipv4 hdr)|
///   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
fn hash_packet(ip_header_start: usize, tcp_header_start: usize, pkt: &[u8]) -> u32 {
    use fnv;
    use std::hash::Hasher;
    let mut h = fnv::FnvHasher::default();
    //let src_dst_ip = &pkt[ip_header_start + 12..ip_header_start + 20];
    let src_dst_port = &pkt[tcp_header_start..tcp_header_start + 2];
    let ipid = &pkt[ip_header_start + 4..ip_header_start + 6];
    //h.write(src_dst_ip);
    h.write(src_dst_port);
    h.write(ipid);
    h.finish() as u32
}

fn main() {
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

    let log = portus::algs::make_logger();

    let iface = matches.value_of("iface").unwrap();
    let filter = matches.value_of("filter").unwrap();
    let mut sample_rate = value_t!(matches.value_of("sample_rate"), u32).unwrap();
    let no_ethernet = matches.is_present("no_ethernet");

    let ip_header_start = if no_ethernet { 0 } else { MAC_HEADER_LENGTH };
    let tcp_header_start = ip_header_start + IP_HEADER_LENGTH;

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
    let recv_sock = sock.try_clone().unwrap();
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

    let (tx, rx): (Sender<(u64, u32, u64)>, Receiver<(u64, u32, u64)>) = mpsc::channel();

    thread::spawn(move || loop {
        let (ts, hash, recvd) = rx.recv().unwrap();
        let msg = OutBoxFeedbackMsg {
            bundle_id: 42,
            marked_packet_hash: hash,
            epoch_bytes: recvd,
            epoch_time: ts,
        };

        sock.send_to(msg.as_bytes().as_slice(), inbox.unwrap())
            .expect("failed to send on UDP socket");
    });

    let mut bytes_recvd: u64 = 0;
    let mut last_bytes_recvd: u64 = 0;
    let mut r1: u64 = 0;
    let mut pkts: u64 = 0;

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
                Err(e) => println!("{:?}", e),
            }
        }
    });

    loop {
        match r.try_recv() {
            Ok(epoch_length_packets) => {
                if epoch_length_packets > 0 {
                    info!(log, "adjust_epoch";
                        "curr" => sample_rate,
                        "new" => epoch_length_packets,
                    );

                    sample_rate = epoch_length_packets;
                }
            }
            Err(mpsc::TryRecvError::Empty) => (),
            Err(mpsc::TryRecvError::Disconnected) => break,
        }

        match cap.next() {
            Ok(pkt) => {
                let now = time::precise_time_ns();
                let data = pkt.data;

                // Is this a TCP packet?
                if data[ip_header_start + PROTO_IN_IP_HEADER] != IP_PROTO_TCP {
                    continue;
                }

                bytes_recvd += pkt.header.len as u64;
                if no_ethernet {
                    bytes_recvd += MAC_HEADER_LENGTH as u64;
                }

                let hash = hash_packet(ip_header_start, tcp_header_start, data);

                pkts += 1;

                // If hash ends in X zeros, "mark" it
                if hash % sample_rate == 0 {
                    let r2 = now;
                    tx.send((r2, hash, bytes_recvd)).unwrap();
                    debug!(log, "outbox hash";
                        "ip" => ?&data[ip_header_start + 12..ip_header_start+20],
                        "ports" => ?&data[tcp_header_start..tcp_header_start+4],
                        "ipid" => ?&data[ip_header_start+4..ip_header_start+6],
                        "hash" => hash,
                    );

                    if r1 != 0 {
                        let recv_epoch_seconds = (r2 - r1) as f64 / 1e9;
                        let recv_epoch_bytes = (bytes_recvd - last_bytes_recvd) as f64;
                        info!(log, "outbox epoch";
                            "recv_rate" => recv_epoch_bytes / recv_epoch_seconds,
                            "recv_epoch_bytes" => recv_epoch_bytes,
                            "recv_epoch_ns" => (r2 - r1),
                            "recv_epoch_packet_count" => pkts,
                        );
                    }

                    r1 = r2;
                    last_bytes_recvd = bytes_recvd;
                    pkts = 0;
                }
            }
            _ => {}
        }
    }
}
