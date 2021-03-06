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

#[derive(Clone, Copy)]
pub struct Epoch {
    pub elapsed_ns: u64,
    pub bytes: u64,
}

#[derive(Default)]
pub struct EpochHistory {
    pub window: usize,
    sending: VecDeque<Epoch>,
    receiving: VecDeque<Epoch>,
}

fn rate<'a>(epochs: impl Iterator<Item = &'a Epoch>) -> f64 {
    let (tot_bytes, tot_elapsed_ns) = epochs
        .map(|e| (e.bytes as f64, e.elapsed_ns as f64))
        .fold((0.0, 0.0), |(b, t), (c_b, c_t)| (b + c_b, t + c_t));
    tot_bytes / (tot_elapsed_ns / 1e9)
}

impl EpochHistory {
    pub fn got_epoch(&mut self, send_epoch: Epoch, recv_epoch: Epoch) -> (f64, f64) {
        self.sending.push_back(send_epoch);
        self.receiving.push_back(recv_epoch);

        assert!(self.window > 0);

        while self.sending.len() > self.window {
            self.sending.pop_front();
        }

        while self.receiving.len() > self.window {
            self.receiving.pop_front();
        }

        (rate(self.sending.iter()), rate(self.receiving.iter()))
    }
}
