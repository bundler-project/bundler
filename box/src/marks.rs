use std::collections::VecDeque;

#[derive(Clone, Copy)]
pub struct MarkedInstant {
    pub time: u64,
    pub pkt_hash: u32,
    pub send_byte_clock: u64,
}

#[derive(Default)]
pub struct MarkHistory {
    marks: VecDeque<MarkedInstant>,
}

impl MarkHistory {
    pub fn insert(&mut self, pkt_hash: u32, time: u64, send_byte_clock: u64) {
        self.marks.push_back(MarkedInstant {
            time,
            pkt_hash,
            send_byte_clock,
        })
    }

    // TODO can implement binary search for perf
    fn find_idx(&self, pkt_hash: u32) -> Option<usize> {
        for i in 0..self.marks.len() {
            if self.marks[i].pkt_hash == pkt_hash {
                return Some(i);
            }
        }

        None
    }

    pub fn get(&mut self, _now: u64, pkt_hash: u32) -> Option<MarkedInstant> {
        let idx = self.find_idx(pkt_hash)?;
        self.marks.drain(0..(idx + 1)).last()
    }
}
