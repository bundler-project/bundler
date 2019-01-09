extern crate bundler;
extern crate clap;
extern crate time;

use clap::{value_t, App, Arg};
use pcap::{Capture, Device};

use bundler::serialize;
use bundler::serialize::OutBoxFeedbackMsg;

use slog::info;
use std::net::UdpSocket;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

// Header lengths
const MAC_HEADER_LENGTH: usize = 14;
const IP_HEADER_LENGTH: usize = 20;
// Locations in headers
const PROTO_IN_IP_HEADER: usize = 10;
const SEQ_IN_TCP_HEADER: usize = 5;
// Values
const IP_PROTO_TCP: u8 = 6;
const SEQ_LENGTH: usize = 4;
// Locations from beginning of packet
const SEQ: usize = MAC_HEADER_LENGTH + IP_HEADER_LENGTH + SEQ_IN_TCP_HEADER - 1;
const PROTO: usize = MAC_HEADER_LENGTH + PROTO_IN_IP_HEADER - 1;

fn hash_packet(buf: &[u8]) -> u32 {
    use fnv;
    use std::hash::Hasher;
    let mut h = fnv::FnvHasher::default();
    h.write(buf);
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
                .takes_value(true)
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
    let proto_offset = if no_ethernet {
        PROTO - MAC_HEADER_LENGTH
    } else {
        PROTO
    };
    let seq_offset = if no_ethernet {
        SEQ - MAC_HEADER_LENGTH
    } else {
        SEQ
    };
    
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
                if data[proto_offset] != IP_PROTO_TCP {
                    continue;
                }

                bytes_recvd += pkt.header.len as u64;
                if no_ethernet {
                    bytes_recvd += MAC_HEADER_LENGTH as u64;
                }

                // Extract the sequence number and hash it
                let hash = hash_packet(
                    &data[seq_offset..(seq_offset + SEQ_LENGTH)]
                );
                // If hash ends in X zeros, "mark" it
                if hash % sample_rate == 0 {
                    let r2 = now;
                    tx.send((r2, hash, bytes_recvd)).unwrap();
                    if r1 != 0 {
                        let recv_epoch_seconds = (r2 - r1) as f64 / 1e9;
                        let recv_epoch_bytes = (bytes_recvd - last_bytes_recvd) as f64;
                        info!(log, "outbox epoch";
                            "recv_rate" => recv_epoch_bytes / recv_epoch_seconds,
                        );
                    }

                    r1 = r2;
                    last_bytes_recvd = bytes_recvd;
                }
            }
            _ => {}
        }
    }
}
