extern crate bundler;
extern crate clap;
extern crate time;

use clap::{value_t, App, Arg};
use pcap::{Capture, Device};

use bundler::serialize::OutBoxFeedbackMsg;

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
// Locations from beginning of packet
const SEQ: usize = MAC_HEADER_LENGTH + IP_HEADER_LENGTH + SEQ_IN_TCP_HEADER - 1;
const PROTO: usize = MAC_HEADER_LENGTH + PROTO_IN_IP_HEADER - 1;

fn adler32(buf: &[u8], len: u8) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    let mut n: usize = 0;
    while (n as u8) < len {
        s1 = (s1 + (buf[n] as u32)) % 65521;
        s2 = (s2 + s1) % 65521;
        n += 1;
    }
    return (s2 << 16) | s1;
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
            Arg::with_name("samplerate")
                .short("s")
                .long("samplerate")
                .help("sample 1 out of every [samplerate] packets")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("inbox")
                .long("inbox")
                .help("address of inbox")
                .takes_value(true)
                .required(true),
        )
        .get_matches();

    let iface = matches.value_of("iface").unwrap();
    let filter = matches.value_of("filter").unwrap();
    let samplerate = value_t!(matches.value_of("samplerate"), u32).unwrap();

    let inbox = matches.value_of("inbox").unwrap().to_owned();
    let sock = UdpSocket::bind("0.0.0.0:34254").expect("failed to create UDP socket");

    let (tx, rx): (Sender<(u64, u32, u32)>, Receiver<(u64, u32, u32)>) = mpsc::channel();

    thread::spawn(move || loop {
        let (ts, hash, recvd) = rx.recv().unwrap();
        let msg = OutBoxFeedbackMsg {
            bundle_id: 42,
            epoch_bytes: recvd,
            marked_packet_hash: hash,
            recv_time: ts,
        };
        sock.send_to(msg.as_bytes().as_slice(), &inbox)
            .expect("failed to send on UDP socket");
    });

    let devs = Device::list().unwrap();
    let dev = devs.into_iter().find(|dev| dev.name == iface);
    let mut cap = Capture::from_device(dev.unwrap())
        .unwrap()
        .promisc(true) // Promiscuous mode because the packets are not destined for our IP
        .timeout(1) // Poll every 1ms
        .snaplen(42) // We only need up to byte 42 to read the sequence number
        .open()
        .unwrap();
    cap.filter(filter).unwrap();

    let mut bytes_recvd: u32 = 0;

    loop {
        match cap.next() {
            Ok(pkt) => {
                let data = pkt.data;
                // Is this a TCP packet?
                if data[PROTO] != IP_PROTO_TCP {
                    continue;
                }
                bytes_recvd += pkt.header.len;
                // Extract the sequence number and hash it
                let hash = adler32(&data[SEQ..(SEQ + 4)], 4);
                // If hash ends in X zeros, "mark" it
                if hash % samplerate == 0 {
                    tx.send((time::precise_time_ns(), hash, bytes_recvd))
                        .unwrap()
                }
            }
            _ => {}
        }
    }
}
