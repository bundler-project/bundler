extern crate bundler;
extern crate clap;
extern crate minion;

#[cfg(target_os = "linux")]
use std::process::Command;

use slog::{error, info, trace, warn};

#[cfg(target_os = "linux")]
use regex::Regex;

#[cfg(target_os = "linux")]
fn lookup_qdisc(logger: &slog::Logger, iface: &str) -> Option<u32> {
    let qdisc_show = Command::new("sudo")
        .args(&["tc", "qdisc", "show", "dev", iface])
        .output()
        .expect("tc qdisc show");

    if qdisc_show.status.success() {
        let qdisc_show_out = String::from_utf8_lossy(&qdisc_show.stdout);
        let re = Regex::new(r"qdisc bundle_inbox ([0-9a-f]{4}): parent 1:2").unwrap();
        let caps = re.captures(&qdisc_show_out);
        if caps.is_some() {
            return Some(u32::from_str_radix(caps.unwrap().get(1).unwrap().as_str(), 16).unwrap());
        } else {
            warn!(
                logger,
                "qdisc show did not match pattern 'qdisc bundle_inbox [0-9a-f]{{4}} parent 1:2'"
            );
        }
    } else {
        warn!(
            logger,
            "qdisc show failed with exit code {}",
            qdisc_show.status.code().unwrap()
        );
    }
    return None;
}

#[cfg(target_os = "linux")]
fn get_iface_ip(logger: &slog::Logger, iface: &str) -> Option<String> {
    let ip_addr = Command::new("ip")
        .arg("addr")
        .arg("show")
        .arg(iface)
        .output()
        .expect("ip addr show");

    if ip_addr.status.success() {
        let ip_addr_out = String::from_utf8_lossy(&ip_addr.stdout);
        let re = Regex::new(&format!(
            "{}{}",
            r"inet ([0-9]{1,3}.[0-9]{1,3}.[0-9]{1,3}.[0-9]{1,3}).*", iface
        ))
        .unwrap();
        let caps = re.captures(&ip_addr_out);
        if caps.is_some() {
            return Some(caps.unwrap().get(1).unwrap().as_str().to_owned());
        } else {
            warn!(
                logger,
                "ip addr show did not match pattern '{}'",
                format!(
                    "{}{}",
                    r"inet ([0-9]{1,3}.[0-9]{1,3}.[0-9]{1,3}.[0-9]{1,3}).*", iface
                )
            );
        }
    } else {
        warn!(
            logger,
            "ip addr show failed with exit code {}",
            ip_addr.status.code().unwrap()
        );
    }
    return None;
}

#[cfg(target_os = "linux")]
fn setup_qdisc(
    logger: &slog::Logger,
    iface: &str,
    self_port: u16,
    verbose: bool,
    matches: clap::ArgMatches,
) -> (u32, u32) {
    use std::path::PathBuf;
    info!(logger, "Installing bundler qdisc"; "interface" => iface);
    if matches.is_present("keep_qdisc") {
        let major_handle = lookup_qdisc(logger, iface);
        if major_handle.is_some() {
            return (major_handle.unwrap(), 0);
        }
        warn!(logger, "Cannot find bundler qdisc, attempting to reinstall");
    }

    let qtype: &str = matches
        .value_of("qtype")
        .expect("Must provide qtype when installing qdisc");
    let buffer: &str = matches
        .value_of("buffer")
        .expect("Must provide buffer size when installing qdisc");

    let qdisc_root_dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "qdisc"].iter().collect();

    let tc_lib_dir = matches.value_of("tc_lib_dir").map_or_else(|| {
        let dir: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
                "qdisc",
                "iproute2",
                "tc"
        ].iter().collect();
        warn!(logger,
            "TC_LIB_DIR not supplied. Assuming TC_LIB_DIR={}. If this is incorrect, please provide --tc_lib_dir",
            dir.to_string_lossy()
        );
        dir
    }, |d| PathBuf::from(d));

    Command::new("sudo")
        .env("TC_LIB_DIR", &tc_lib_dir)
        .arg("-E")
        .args(&["tc", "qdisc", "del", "dev", iface, "root"])
        .output()
        .expect("tc qdisc del");

    Command::new("sudo")
        .arg("rmmod")
        .arg("sch_bundle_inbox")
        .output()
        .expect("rmmod");

    let mut make = Command::new("make");
    make.arg(format!("QTYPE={}", qtype))
        .current_dir(&qdisc_root_dir);
    if verbose {
        make.arg("VERBOSE_LOGGING=y");
    }

    let out = make.output().expect(&format!("make QTYPE={}", qtype));
    if !out.status.success() {
        warn!(logger, "qdisc make failed");
        println!("{}", std::str::from_utf8(&out.stdout).unwrap());
    }

    Command::new("sudo")
        .arg("insmod")
        .arg("sch_bundle_inbox.ko")
        .current_dir(&qdisc_root_dir)
        .output()
        .expect("insmod");

    info!(logger, "Loaded qdisc kernel module");

    let ok = Command::new("sudo")
        .args(&[
            "tc", "qdisc", "add", "dev", iface, "root", "handle", "1:", "prio", "bands", "3",
        ])
        .output()
        .expect("tc qdisc add root prio");

    if !ok.status.success() {
        let stdout = std::str::from_utf8(&ok.stdout).unwrap();
        let stderr = std::str::from_utf8(&ok.stderr).unwrap();
        println!("{}", stdout);
        println!("{}", stderr);
        error!(logger, "tc qdisc add root prio");
    }

    let ok = Command::new("sudo")
        .args(&[
            "tc",
            "qdisc",
            "add",
            "dev",
            iface,
            "parent",
            "1:1",
            "pfifo_fast",
        ])
        .output()
        .expect("tc qdisc add child pfifo");

    if !ok.status.success() {
        let stdout = std::str::from_utf8(&ok.stdout).unwrap();
        let stderr = std::str::from_utf8(&ok.stderr).unwrap();
        println!("{}", stdout);
        println!("{}", stderr);
        error!(logger, "tc qdisc add root prio");
    }

    let sip = match matches.value_of("sip") {
        Some(ip) => ip.to_owned(),
        None => get_iface_ip(logger, iface).expect("failed to find iface ip"),
    };

    info!(logger, "Reset qdisc"; "ip" => &sip);
    Command::new("sudo")
        .args(&[
            "tc",
            "filter",
            "add",
            "dev",
            iface,
            "parent",
            "1:",
            "protocol",
            "ip",
            "prio",
            "1",
            "u32", // give priority 1 (highest)
            "match",
            "ip",
            "protocol",
            "17",
            "0xff", // match UDP
            "match",
            "ip",
            "sport",
            &self_port.to_string(),
            "0xffff", // match inbox feedback source port
            "match",
            "ip",
            "src",
            &sip, // match inbox feedback source ip
            "flowid",
            "1:1", // send to child 1 (pfifo_fast)
        ])
        .output()
        .expect("tc filter self -> pfifo");

    let ok = Command::new("sudo")
        .env("TC_LIB_DIR", &tc_lib_dir)
        .arg("-E")
        .args(&[
            "tc",
            "qdisc",
            "add",
            "dev",
            iface,
            "parent",
            "1:2",
            "bundle_inbox",
            "rate",
            "100mbit",
            "burst",
            "1mbit",
            "limit",
            buffer,
        ])
        .output()
        .expect("tc qdisc add child bundler");
    if !ok.status.success() {
        let stdout = std::str::from_utf8(&ok.stdout).unwrap();
        let stderr = std::str::from_utf8(&ok.stderr).unwrap();
        println!("{}", stdout);
        println!("{}", stderr);
        error!(logger, "tc qdisc add child bundler");
    }

    let major_handle = lookup_qdisc(logger, iface);
    if major_handle.is_none() {
        panic!("Failed to find qdisc");
    } else {
        return (major_handle.unwrap(), 0);
    }
}

struct Prio {
    port: Option<u16>,
    log: slog::Logger,
}

impl bundler::prio::Prioritizer for Prio {
    fn assign_priority(&mut self, flow: bundler::prio::FlowInfo) -> u16 {
        trace!(self.log, "Flow Priority"; "flow" => ?flow);

        if let Some(p) = self.port {
            if flow.dst_port == p {
                2
            } else {
                1
            }
        } else {
            1
        }
    }
}

fn setup_prioritizer(log: &slog::Logger, port: Option<u16>) -> Prio {
    Prio {
        port,
        log: log.clone(),
    }
}

#[cfg(target_os = "linux")]
fn main() {
    use clap::{value_t, App, Arg};
    let matches = App::new("inbox")
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
            Arg::with_name("port")
                .short("p")
                .long("port")
                .help("UDP port to listen on for messages from outbox")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("sample_rate")
                .short("s")
                .long("sample_rate")
                .help("Number of times in each pipe-size batch we should mark a packet")
                .default_value("100")
                .required(true),
        )
        .arg(
            Arg::with_name("dynamic_sample_rate")
                .short("d")
                .long("use_dynamic_sample_rate")
                .help("Whether to dynamically adjust the sample rate")
                .default_value("true"),
        )
        .arg(
            Arg::with_name("outbox")
                .long("outbox")
                .help("address of outbox")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("tc_lib_dir")
                .long("tc_lib_dir")
                .help("path to custom tc for talking to bundler qdisc")
                .takes_value(true)
                .required(false)
        )
        .arg(
            Arg::with_name("keep_qdisc")
                .long("keep_qdisc")
                .help("unless this flag is supplied, the qdisc is always cleared and re-inserted when inbox starts")
                .required(false)
        )
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .long("verbose")
                .required(false)
        )
        .arg(
            Arg::with_name("qtype")
                .short("q")
                .long("qtype")
                .takes_value(true)
                .required(false)
        )
        .arg(
            Arg::with_name("buffer")
                .long("buffer")
                .takes_value(true)
                .required(false)
        )
        .arg(
            Arg::with_name("prio_port")
                .long("prio_port")
                .takes_value(true)
                .required(false)
        )
        .arg(
            Arg::with_name("sip")
                .long("sip")
                .takes_value(true)
                .required(false)
                .help("inbox source IP address, defaults to the ip address of the provided interface, only provide if different")
        )
        .get_matches();

    let iface = String::from(matches.value_of("iface").unwrap());
    let listen_port = value_t!(matches.value_of("port"), u16).unwrap();
    let sample_rate = matches.value_of("sample_rate").unwrap().parse().unwrap();
    let dynamic_sample_rate = matches
        .value_of("dynamic_sample_rate")
        .unwrap()
        .parse::<bool>()
        .unwrap();
    let outbox = matches.value_of("outbox").map(String::from);
    let port = value_t!(matches.value_of("prio_port"), u16).ok();

    let log = portus::algs::make_logger();

    let verbose = matches.is_present("verbose");

    let (handle_major, handle_minor) = setup_qdisc(&log, &iface, listen_port, verbose, matches);
    let prio = setup_prioritizer(&log, port);

    use bundler::inbox::Runtime;
    use minion::Cancellable;
    let mut r = Runtime::new(
        log,
        Some(prio),
        listen_port,
        outbox,
        iface,
        (handle_major, handle_minor),
        dynamic_sample_rate,
        sample_rate,
    )
    .unwrap();
    r.run().unwrap()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("Runs on Linux only")
}
