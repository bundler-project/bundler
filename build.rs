extern crate bindgen;

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-link-lib=nl-genl-3");
    println!("cargo:rustc-link-lib=nfnetlink");
    println!("cargo:rustc-link-lib=nl-route-3");
    println!("cargo:rustc-link-lib=nl-3");

    let nl_bindings = bindgen::Builder::default()
            .header("nl-route.h")
            .clang_arg("-I/usr/include/libnl3")
            .whitelist_function("nl_socket_alloc")
            .whitelist_function("nl_connect")
            .whitelist_function("rtnl_link_alloc_cache")
            .whitelist_function("rtnl_link_get_by_name")
            .whitelist_function("rtnl_link_get_ifindex")
            .whitelist_function("rtnl_qdisc_alloc_cache")
            .whitelist_function("rtnl_qdisc_alloc")
            .whitelist_function("rtnl_qdisc_get")
            .whitelist_function("rtnl_tc_get_stat")
            .whitelist_function("rtnl_qdisc_put")
            .whitelist_function("rtnl_qdisc_add")
            .whitelist_function("TC_CAST")
            .whitelist_function("TC_HANDLE")
            .whitelist_function("rtnl_qdisc_tbf_set_rate")
            .whitelist_var("NETLINK_ROUTE")
            .whitelist_var("AF_UNSPEC")
            .whitelist_var("NLM_F_REPLACE")
            .generate()
            .expect("unable to generate netlink-route bindings");
    let nl_out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    nl_bindings
        .write_to_file(nl_out_path.join("libnl.rs"))
        .expect("Unable to write libnl bindings");

    let mut libccp_make = std::process::Command::new("make")
        .current_dir("./libccp")
        .spawn()
        .expect("libccp make failed");
    libccp_make.wait().expect("libccp make spawned but failed");

    println!("cargo:rustc-link-search=./libccp");
    println!("cargo:rustc-link-lib=ccp");

    let ccp_bindings = bindgen::Builder::default()
        .header("./libccp/ccp.h")
        .whitelist_function(r#"ccp_\w+"#)
        .blacklist_type(r#"u\d+"#)
        .rustfmt_bindings(true)
        .generate()
        .expect("Unable to generate bindings");

    let ccp_out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    ccp_bindings
        .write_to_file(ccp_out_path.join("libccp.rs"))
        .expect("Unable to write libccp bindings")
}
