//! API for specifying priority policy

#[derive(Debug, Clone)]
pub struct FlowInfo {
    pub src_ip: std::net::Ipv4Addr,
    pub src_port: u16,
    pub dst_ip: std::net::Ipv4Addr,
    pub dst_port: u16,
}

impl FlowInfo {
    pub fn new(src_ip: u32, src_port: u16, dst_ip: u32, dst_port: u16) -> Self {
        Self {
            src_ip: std::net::Ipv4Addr::from(src_ip),
            src_port,
            dst_ip: std::net::Ipv4Addr::from(dst_ip),
            dst_port,
        }
    }
}

pub trait Prioritizer {
    fn assign_priority(&mut self, _flow: FlowInfo) -> u16 {
        1
    }
}
