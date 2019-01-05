use std;

use super::nl::*;
use super::serialize::QDiscUpdateMsg;
use portus::ipc;
use portus::ipc::netlink;
use portus::ipc::Ipc;
use slog;
use slog::debug;

pub struct Qdisc {
    logger: slog::Logger,
    rtnl_sock: *mut nl_sock,
    qdisc: *mut rtnl_qdisc,
    update_sock: netlink::Socket<ipc::Blocking>,
}

use std::ffi::CString;
impl Qdisc {
    pub fn bind(logger: slog::Logger, if_name: String, (tc_maj, tc_min): (u32, u32)) -> Self {
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
            }
        }
    }

    pub fn set_rate(&mut self, rate: u32, burst: u32) -> Result<(), ()> {
        debug!(self.logger, "set rate"; "rate" => rate, "burst" => burst);
        unsafe {
            rtnl_qdisc_tbf_set_rate(self.qdisc, rate as i32, burst as i32, 0);
            let ret = rtnl_qdisc_add(self.rtnl_sock, self.qdisc, NLM_F_REPLACE as i32);
            if ret < 0 {
                return Err(());
            }
            Ok(())
        }
    }

    pub fn set_epoch_length(&self, epoch_length_packets: u32) -> Result<(), portus::Error> {
        let msg = QDiscUpdateMsg {
            bundle_id: 42,
            sample_rate: epoch_length_packets,
        };

        self.update_sock.send(&msg.as_bytes())
    }
}
impl Drop for Qdisc {
    fn drop(&mut self) {
        unsafe {
            rtnl_qdisc_put(self.qdisc);
        }
    }
}
