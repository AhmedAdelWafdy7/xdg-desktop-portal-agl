// Minimal screenshot CLI for validating the wayland_capture backends against a live compositor.
//
// Usage: cargo run --example capture -- [output-name] [out.png]
// Connects to $WAYLAND_DISPLAY, captures the selected (or first) output, and writes a PNG.

use wayland_capture::{OutputSelector, capture_output, probe};

fn main() {
    tracing_subscriber::fmt::init();

    let mut args = std::env::args().skip(1);
    let output_arg = args.next();
    let path = args.next().unwrap_or_else(|| "capture.png".to_string());

    // Report what the compositor advertises before capturing.
    let conn = match probe::connect() {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("probe failed: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("connected. capabilities:");
    eprintln!(
        "  weston_capture_v1:  {:?}",
        conn.capabilities.weston_screenshooter
    );
    eprintln!(
        "  agl_screenshooter:  {:?}",
        conn.capabilities.agl_screenshooter
    );
    eprintln!(
        "  wlr_screencopy:     {:?}",
        conn.capabilities.zwlr_screencopy_manager
    );
    eprintln!(
        "  selected backend:   {:?}",
        conn.capabilities.selected_capture_backend()
    );
    eprintln!("  outputs:");
    for (i, o) in conn.outputs.iter().enumerate() {
        eprintln!("    [{i}] name={:?} {}x{}", o.name, o.width, o.height);
    }

    // Pick the requested output; if the name isn't found (e.g. the compositor doesn't send
    // wl_output.name), fall back to the first output instead of failing.
    let selector = match output_arg {
        Some(name)
            if conn
                .select_output(&OutputSelector::Name(name.clone()))
                .is_some() =>
        {
            OutputSelector::Name(name)
        }
        Some(name) => {
            eprintln!("output {name:?} not found; falling back to first output");
            OutputSelector::First
        }
        None => OutputSelector::First,
    };

    match capture_output(&selector) {
        Ok(buffer) => {
            eprintln!(
                "captured {}x{} stride={} format={:?}",
                buffer.width, buffer.height, buffer.stride, buffer.format
            );
            let png = buffer.encode_png().expect("encode png");
            std::fs::write(&path, &png).expect("write png");
            eprintln!("wrote {} ({} bytes)", path, png.len());
        }
        Err(e) => {
            eprintln!("capture failed: {e}");
            std::process::exit(1);
        }
    }
}
