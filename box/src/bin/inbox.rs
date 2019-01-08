extern crate bundler;
extern crate clap;
extern crate minion;

use bundler::Runtime;
use clap::{value_t, App, Arg};
use minion::Cancellable;

fn main() {
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
            Arg::with_name("handle_major")
                .short("M")
                .long("handle_major")
                .help("Major handle of inbox qdisc")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("handle_minor")
                .short("m")
                .long("handle_minor")
                .help("Minor handle of inbox qdisc")
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
        .get_matches();

    let iface = String::from(matches.value_of("iface").unwrap());
    let listen_port = value_t!(matches.value_of("port"), u16).unwrap();
    let handle_major = {
        let major = matches.value_of("handle_major").unwrap();
        if major.starts_with("0x") {
            u32::from_str_radix(major.split_at(2).1, 16)
        } else if major.starts_with("x") {
            u32::from_str_radix(major.split_at(1).1, 16)
        } else {
            u32::from_str_radix(major, 10)
        }
    }
    .unwrap();
    let handle_minor = {
        let minor = matches.value_of("handle_minor").unwrap();
        if minor.starts_with("0x") {
            u32::from_str_radix(minor.split_at(2).1, 16)
        } else if minor.starts_with("x") {
            u32::from_str_radix(minor.split_at(1).1, 16)
        } else {
            u32::from_str_radix(minor, 10)
        }
    }
    .unwrap();
    let sample_rate = matches.value_of("sample_rate").unwrap().parse().unwrap();
    let dynamic_sample_rate = matches
        .value_of("dynamic_sample_rate")
        .unwrap()
        .parse()
        .unwrap();
    let mut r = Runtime::new(
        listen_port,
        iface,
        (handle_major, handle_minor),
        dynamic_sample_rate,
        sample_rate,
    )
    .unwrap();
    r.run().unwrap()
}
