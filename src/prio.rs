//! API for specifying priority policy

#[derive(Debug, Clone, Copy)]
pub struct FlowInfo {
    pub src_ip: u32,
    pub src_port: u16,
    pub dst_ip: u32,
    pub dst_port: u16,
}

pub trait Prioritizer {
    fn assign_priority(&mut self, _flow: FlowInfo) -> u16 {
        1
    }
}
