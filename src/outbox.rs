use slog::{debug, info};
use std::sync::mpsc;

use crate::hash;
use crate::{IP_HEADER_LENGTH, IP_PROTO_TCP, MAC_HEADER_LENGTH, PROTO_IN_IP_HEADER};

pub fn start_outbox<T: pcap::Activated + ?Sized>(
    mut cap: pcap::Capture<T>,
    tx: crossbeam::Sender<(u64, u32, u64)>,
    r: mpsc::Receiver<u32>,
    mut sample_rate: u32,
    no_ethernet: bool,
    log: slog::Logger,
) -> Result<(), ()> {
    let ip_header_start = if no_ethernet { 0 } else { MAC_HEADER_LENGTH };
    let tcp_header_start = ip_header_start + IP_HEADER_LENGTH;
    let mut bytes_recvd: u64 = 0;
    let mut last_bytes_recvd: u64 = 0;
    let mut r1: u64 = 0;
    let mut pkts: u64 = 0;

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
            Err(mpsc::TryRecvError::Disconnected) => unreachable!(),
        }

        match cap.next() {
            Ok(pkt) => {
                let now = pkt.header.ts;
                let now = now.tv_sec as u64 * 1_000_000_000 + now.tv_usec as u64 * 1_000; // ns since epoch
                let data = pkt.data;

                // Is this a TCP packet?
                if data[ip_header_start + PROTO_IN_IP_HEADER] != IP_PROTO_TCP {
                    continue;
                }

                bytes_recvd += pkt.header.len as u64;
                if no_ethernet {
                    bytes_recvd += MAC_HEADER_LENGTH as u64;
                }

                let hash = hash::hash_packet(ip_header_start, tcp_header_start, data);
                pkts += 1;

                // If hash ends in X zeros, "mark" it
                if hash % sample_rate == 0 {
                    let r2 = now;
                    tx.send((r2, hash, bytes_recvd)).unwrap();
                    debug!(log, "outbox hash";
                        "ip" => ?hash::unpack_ips(data, ip_header_start),
                        "ports" => ?hash::unpack_ports(data, tcp_header_start),
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
            _ => {
                return Err(());
            }
        }
    }
}
