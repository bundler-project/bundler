pub fn get_epoch_length(rate_bytes: f64, rtt_sec: f64) -> u32 {
    let inflight_bdp = rate_bytes * rtt_sec / 1500.0;
    // round to power of 2
    let inflight_bdp_rounded = crate::round_down_power_of_2(inflight_bdp as u32);

    std::cmp::min(std::cmp::max(inflight_bdp_rounded >> 2, 4), 1024)
}

pub trait Datapath {
    fn set_approx_cwnd(&mut self, cwnd_bytes: u32) -> Result<(), ()>;
    fn set_rate(&mut self, rate: u32) -> Result<(), ()>;
    fn update_rtt(&mut self, rtt_ns: u64) -> Result<(), ()>;
    fn set_epoch_length(&mut self, epoch_length_packets: u32) -> Result<(), portus::Error>;
    fn update_send_rate(&mut self, observed_sending_bytes_per_sec: u64);
    fn get_curr_epoch_length(&self) -> u32;
}

#[cfg(target_os = "linux")]
pub mod qdisc;
