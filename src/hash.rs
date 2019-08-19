/// Take the FNV hash of the packet for epoch boundary identification.
/// Use (src ip, dst ip, src port, dst port, IP ID) to identify packets.
///
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
pub fn hash_packet(ip_header_start: usize, tcp_header_start: usize, pkt: &[u8]) -> u32 {
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

pub fn unpack_ips(pkt: &[u8], ip_header_start: usize) -> (std::net::Ipv4Addr, std::net::Ipv4Addr) {
    (
        std::net::Ipv4Addr::from(*arrayref::array_ref!(pkt, ip_header_start + 12, 4)),
        std::net::Ipv4Addr::from(*arrayref::array_ref!(pkt, ip_header_start + 16, 4)),
    )
}

pub fn unpack_ports(pkt: &[u8], tcp_header_start: usize) -> (u16, u16) {
    use bytes::{ByteOrder, LittleEndian};
    (
        LittleEndian::read_u16(&pkt[tcp_header_start..tcp_header_start + 2]),
        LittleEndian::read_u16(&pkt[tcp_header_start + 2..tcp_header_start + 4]),
    )
}