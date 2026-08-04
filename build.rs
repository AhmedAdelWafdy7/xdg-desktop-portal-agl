fn main() {
    println!("cargo:rerun-if-changed=data/");
    println!("cargo:rerun-if-changed=build.rs");
    // wayland-scanner expands the protocol XML at compile time
    println!("cargo:rerun-if-changed=src/wayland_capture/protocols/agl/");
    println!("cargo:rerun-if-changed=src/wayland_capture/protocols/weston/");
}
