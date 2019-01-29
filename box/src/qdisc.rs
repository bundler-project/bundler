use std;

use super::nl::*;
use super::serialize;
use super::serialize::QDiscUpdateMsg;
use portus::ipc;
use portus::ipc::netlink;
use portus::ipc::Ipc;
use slog;
use slog::info;
use std::cmp::min;

pub struct Qdisc {
    logger: slog::Logger,
    rtnl_sock: *mut nl_sock,
    qdisc: *mut rtnl_qdisc,
    update_sock: netlink::Socket<ipc::Blocking>,
    outbox_report: std::net::UdpSocket,
    outbox_addr: Option<std::net::SocketAddr>,
    outbox_found: std::sync::mpsc::Receiver<std::net::SocketAddr>,
    rtt_ns: u64,
    min_rtt_ns: u64,
    observed_sending_bytes_per_sec: u64,
    rate_bytes_per_sec: u32,
    cwnd_bytes: u32,
    curr_set_rate: u32,
    use_dynamic_epoch: bool,
    curr_epoch_length: u32,
}

fn get_epoch_length(rate_bytes: f64, rtt_sec: f64) -> u32 {
    let inflight_bdp = rate_bytes * rtt_sec / 1500.0;
    // round to power of 2
    let inflight_bdp_rounded = super::round_down_power_of_2(inflight_bdp as u32);

    std::cmp::max(inflight_bdp_rounded >> 2, 4)
}

use std::ffi::CString;
impl Qdisc {
    pub fn bind(
        logger: slog::Logger,
        if_name: String,
        (tc_maj, tc_min): (u32, u32),
        use_dynamic_epoch: bool,
        outbox_found_rx: std::sync::mpsc::Receiver<std::net::SocketAddr>,
        outbox_report: std::net::UdpSocket,
    ) -> Self {
        unsafe {
            let mut all_links: *mut nl_cache = std::mem::uninitialized();
            let mut all_qdiscs: *mut nl_cache = std::mem::uninitialized();

            let rtnl_sock = nl_socket_alloc();
            nl_connect(rtnl_sock, NETLINK_ROUTE as i32);

            let ret = rtnl_link_alloc_cache(rtnl_sock, AF_UNSPEC as i32, &mut all_links);
            if ret < 0 {
                panic!(format!("rtnl_link_alloc_cache failed: {}", ret));
            }

            let link = rtnl_link_get_by_name(all_links, CString::new(if_name).unwrap().as_ptr());
            let ifindex = rtnl_link_get_ifindex(link);

            // println!("nitems={:#?}", nl_cache_nitems(all_qdiscs));
            //println!("first={:#?}", nl_cache_get_first(all_qdiscs));

            let ret2 = rtnl_qdisc_alloc_cache(rtnl_sock, &mut all_qdiscs);
            if ret2 < 0 {
                panic!(format!("rtnl_qdisc_alloc_cache failed: {}", ret2));
            }
            let tc_handle = ((tc_maj << 16) & 0xFFFF0000) | (tc_min & 0x0000FFFF);
            let qdisc = rtnl_qdisc_get(all_qdiscs, ifindex, tc_handle);
            if qdisc.is_null() {
                panic!("rtnl_qdisc_get failed")
            }

            let update_sock = netlink::Socket::<ipc::Blocking>::new().unwrap();

            Qdisc {
                logger,
                rtnl_sock,
                qdisc,
                update_sock,
                outbox_report,
                outbox_addr: None,
                outbox_found: outbox_found_rx,
                rtt_ns: 0x3fff_ffff,
                min_rtt_ns: 0x3fff_ffff,
                observed_sending_bytes_per_sec: 0x3fff_ffff,
                rate_bytes_per_sec: 0x3fff_ffff,
                cwnd_bytes: 0x3fff_ffff,
                curr_set_rate: 0x3fff_ffff,
                use_dynamic_epoch,
                curr_epoch_length: 4,
            }
        }
    }

    pub fn set_approx_cwnd(&mut self, cwnd_bytes: u32) -> Result<(), ()> {
        if cwnd_bytes / 1500 == 0 {
            return Err(());
        }

        info!(self.logger, "set cwnd"; "cwnd_pkts" => cwnd_bytes / 1500);
        self.cwnd_bytes = cwnd_bytes;
        self.__set_rate()
    }

    pub fn set_rate(&mut self, rate: u32) -> Result<(), ()> {
        info!(self.logger, "set rate"; "rate" => rate);
        self.rate_bytes_per_sec = rate;
        self.__set_rate()
    }

    fn check_outbox_found(&mut self) {
        match self.outbox_found.try_recv() {
            Ok(addr) => self.outbox_addr = Some(addr),
            Err(_) => return,
        }
    }

    pub fn set_epoch_length(&mut self, epoch_length_packets: u32) -> Result<(), portus::Error> {
        if !self.use_dynamic_epoch && self.curr_epoch_length > 0 {
            return Ok(());
        }

        if self.curr_epoch_length == epoch_length_packets {
            return Ok(());
        }

        info!(self.logger, "adjust_epoch";
            "curr" => self.curr_epoch_length,
            "new" => epoch_length_packets,
        );

        self.curr_epoch_length = epoch_length_packets;
        self.check_outbox_found();
        if let Some(addr) = self.outbox_addr {
            // tell the outbox what the epoch length is
            let msg = serialize::OutBoxReportMsg {
                bundle_id: 42,
                epoch_length_packets,
            };

            self.outbox_report
                .send_to(&msg.as_bytes(), addr)
                .unwrap_or_else(|_| 0);
        }

        let msg = QDiscUpdateMsg {
            bundle_id: 42,
            sample_rate: epoch_length_packets,
        };

        self.update_sock.send(&msg.as_bytes())
    }

    pub fn update_rtt(&mut self, rtt_ns: u64) -> Result<(), ()> {
        self.rtt_ns = rtt_ns;
        self.min_rtt_ns = min(self.min_rtt_ns, rtt_ns);
        self.__set_rate()
    }

    pub fn update_send_rate(&mut self, observed_sending_bytes_per_sec: u64) {
        self.observed_sending_bytes_per_sec = observed_sending_bytes_per_sec;
        let epoch_length = get_epoch_length(
            self.observed_sending_bytes_per_sec as f64,
            self.min_rtt_ns as f64 / 1e9,
        );
        self.set_epoch_length(epoch_length).unwrap_or_else(|_| ());
    }

    pub fn get_curr_epoch_length(&self) -> u32 {
        self.curr_epoch_length
    }

    fn __set_rate(&mut self) -> Result<(), ()> {
        if self.cwnd_bytes == 0x3fff_ffff && self.rate_bytes_per_sec == 0x3fff_ffff {
            return Ok(());
        }

        let rtt_sec = self.rtt_ns as f64 / 1e9;
        let cwnd_effective_rate = self.cwnd_bytes as f64 / rtt_sec;
        let rate = std::cmp::min(self.rate_bytes_per_sec, cwnd_effective_rate as u32);

        if rate == self.curr_set_rate {
            return Ok(());
        }

        self.curr_set_rate = rate;
        info!(self.logger, "__set_rate";
            "set_rate" => rate,
            "cwnd_effective_rate" => cwnd_effective_rate,
            "rate" => self.rate_bytes_per_sec,
        );

        unsafe {
            // TODO set burst dynamically
            rtnl_qdisc_tbf_set_rate(
                self.qdisc,
                if rate > 125_000 { rate as i32 } else { 125_000 },
                100_000,
                0,
            );
            let ret = rtnl_qdisc_add(self.rtnl_sock, self.qdisc, NLM_F_REPLACE as i32);
            if ret < 0 {
                return Err(());
            }
            Ok(())
        }
    }
}
impl Drop for Qdisc {
    fn drop(&mut self) {
        unsafe {
            rtnl_qdisc_put(self.qdisc);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_round() {
        let x: f64 = 2.0 * 270.0;
        assert_eq!(super::round_down_power_of_2(x as u32), 512);
        assert_eq!(super::round_down_power_of_2(538), 512);
        assert_eq!(super::round_down_power_of_2(16), 16);
        assert_eq!(super::round_down_power_of_2(1), 1);
        assert_eq!(super::round_down_power_of_2(0), 0);
    }
}
