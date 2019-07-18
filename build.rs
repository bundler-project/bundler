extern crate bindgen;

use std::env;
use std::path::Path;
use std::path::PathBuf;

fn main() {
    std::process::Command::new("./build_tc.sh")
        .current_dir("./qdisc")
        .spawn()
        .expect("build_tc");

    println!("cargo:rustc-link-lib=nfnetlink");

    if !Path::new("./libnl/lib/.libs/libnl-genl-3.a").exists() {
        let mut libnl_make = std::process::Command::new("sh")
            .arg("-c")
            .arg("./autogen.sh && ./configure && make -j")
            .current_dir("./libnl")
            .spawn()
            .expect("libnl make failed");
        libnl_make.wait().expect("libnl make spawned but failed");
    }

    println!("cargo:rustc-link-search=./libnl/lib/.libs/");
    println!("cargo:rustc-link-lib=static=nl-genl-3");
    println!("cargo:rustc-link-lib=static=nl-route-3");
    println!("cargo:rustc-link-lib=static=nl-3");
    /*
    println!("cargo:rustc-link-lib=nl-genl-3");
    println!("cargo:rustc-link-lib=nfnetlink");
    println!("cargo:rustc-link-lib=nl-route-3");
    println!("cargo:rustc-link-lib=nl-3")
    */

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
        .whitelist_function("nl_object_get_type")
        .whitelist_function("nl_cache_get_first")
        .whitelist_function("nl_cache_nitems")
        .whitelist_var("NETLINK_ROUTE")
        .whitelist_var("AF_UNSPEC")
        .whitelist_var("NLM_F_REPLACE")
        .generate()
        .expect("unable to generate netlink-route bindings");
    let nl_out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    nl_bindings
        .write_to_file(nl_out_path.join("libnl.rs"))
        .expect("Unable to write libnl bindings");
}
