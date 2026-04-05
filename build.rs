use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=data/");
    println!("cargo:rerun-if-changed=build.rs");
    let prefix = env::var("PREFIX").unwrap_or_else(|_| "/usr".to_string());
    let datadir = env::var("DATADIR")
        .unwrap_or_else(|_| format!("{}/share", prefix));
    let libexecdir = env::var("LIBEXECDIR")
        .unwrap_or_else(|_| format!("{}/libexec", prefix));

    let install_dirs = format!(
        "pub const PREFIX: &str = \"{}\";\n\
         pub const DATADIR: &str = \"{}\";\n\
         pub const LIBEXECDIR: &str = \"{}\";\n",
        prefix, datadir, libexecdir
    );
    fs::write(out_dir.join("install_dirs.rs"), install_dirs).unwrap();

    let service_in = fs::read_to_string("data/xdg-desktop-portal-agl.service.in")
        .expect("Cannot read service.in");

    let service_out = service_in
        .replace("@libexecdir@", &libexecdir)
        .replace("@prefix@", &prefix);

    fs::write(
        out_dir.join("xdg-desktop-portal-agl.service"),
        service_out,
    ).unwrap();

    println!("cargo:rerun-if-changed=data/xdg-desktop-portal-agl.service.in");
}