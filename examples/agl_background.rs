// Minimal agl_shell client: paints a colored gradient as the output background, then calls
// agl_shell.ready() to lift the compositor's black curtain. Stays alive so the background
// remains visible while you capture a screenshot from another process.
//
// Usage: cargo run --example agl_background -- [width] [height]
// Defaults to 1024x768 (the nested agl-compositor's default output size).

use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use wayland_client::{
    Connection, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_output::WlOutput,
        wl_registry::WlRegistry,
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};

use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::XdgToplevel,
    xdg_wm_base::{self, XdgWmBase},
};

use wayland_capture::protocols::agl_shell::client::agl_shell::{self, AglShell};

struct State {
    bound: Option<bool>,
    configured: bool,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let width: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let height: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(768);

    let conn = Connection::connect_to_env().expect("connect to Wayland ($WAYLAND_DISPLAY)");
    let (globals, mut event_queue) = registry_queue_init::<State>(&conn).expect("registry init");
    let qh = event_queue.handle();
    let registry = globals.registry();
    let contents = globals.contents().clone_list();

    let mut compositor: Option<WlCompositor> = None;
    let mut shm: Option<WlShm> = None;
    let mut agl_shell: Option<AglShell> = None;
    let mut xdg_wm_base: Option<XdgWmBase> = None;
    let mut output: Option<WlOutput> = None;

    for g in &contents {
        match g.interface.as_str() {
            "wl_compositor" => {
                compositor =
                    Some(registry.bind::<WlCompositor, _, _>(g.name, g.version.min(4), &qh, ()));
            }
            "wl_shm" => {
                shm = Some(registry.bind::<WlShm, _, _>(g.name, g.version.min(1), &qh, ()));
            }
            "agl_shell" => {
                agl_shell =
                    Some(registry.bind::<AglShell, _, _>(g.name, g.version.min(11), &qh, ()));
            }
            "xdg_wm_base" => {
                xdg_wm_base =
                    Some(registry.bind::<XdgWmBase, _, _>(g.name, g.version.min(3), &qh, ()));
            }
            "wl_output" if output.is_none() => {
                output = Some(registry.bind::<WlOutput, _, _>(g.name, g.version.min(4), &qh, ()));
            }
            _ => {}
        }
    }

    let compositor = compositor.expect("compositor missing");
    let shm = shm.expect("wl_shm missing");
    let agl_shell = agl_shell.expect("agl_shell missing (is this agl-compositor?)");
    let xdg_wm_base = xdg_wm_base.expect("xdg_wm_base missing");
    let output = output.expect("no wl_output");

    let mut state = State {
        bound: None,
        configured: false,
    };
    // agl_shell (v2+) reports bound_ok/bound_fail after binding.
    event_queue.roundtrip(&mut state).expect("roundtrip bind");
    if state.bound == Some(false) {
        eprintln!("agl_shell bind rejected (another shell client already owns it?)");
        std::process::exit(1);
    }

    // Paint a gradient into a wl_shm buffer (XRGB8888 memory order is [B, G, R, X]).
    let stride = width * 4;
    let size = (stride * height) as usize;
    let fd = unsafe { libc::memfd_create(c"agl-bg".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0, "memfd_create");
    unsafe { assert_eq!(libc::ftruncate(fd, size as libc::off_t), 0) };
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    assert_ne!(ptr, libc::MAP_FAILED, "mmap");

    let pixels = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, size) };
    for y in 0..height {
        for x in 0..width {
            let i = (y * stride + x * 4) as usize;
            pixels[i] = (y * 255 / height) as u8; // B
            pixels[i + 1] = 0x40; // G
            pixels[i + 2] = (x * 255 / width) as u8; // R
            pixels[i + 3] = 0xff; // X
        }
    }

    let owned_fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let borrowed = unsafe { BorrowedFd::borrow_raw(owned_fd.as_raw_fd()) };
    let pool = shm.create_pool(borrowed, size as i32, &qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, Format::Xrgb8888, &qh, ());

    // AGL requires the background to be a "desktop surface": give it an xdg_surface + toplevel
    // role before handing it to agl_shell.set_background.
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &qh, ());
    let _xdg_toplevel = xdg_surface.get_toplevel(&qh, ());

    agl_shell.set_background(&surface, &output);

    // Initial empty commit -> compositor sends xdg_surface.configure, which we ack.
    surface.commit();
    while !state.configured {
        event_queue
            .blocking_dispatch(&mut state)
            .expect("configure");
    }

    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, width, height);
    surface.commit();
    agl_shell.ready();

    event_queue
        .roundtrip(&mut state)
        .expect("roundtrip present");
    eprintln!("background set on {width}x{height}; ready() sent. Capture now (Ctrl-C to stop).");

    loop {
        event_queue
            .blocking_dispatch(&mut state)
            .expect("dispatch loop");
    }
}

impl wayland_client::Dispatch<AglShell, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &AglShell,
        event: agl_shell::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            agl_shell::Event::BoundOk => state.bound = Some(true),
            agl_shell::Event::BoundFail => state.bound = Some(false),
            _ => {}
        }
    }
}

impl wayland_client::Dispatch<XdgWmBase, ()> for State {
    fn event(
        _state: &mut Self,
        proxy: &XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl wayland_client::Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.configured = true;
        }
    }
}

macro_rules! empty_dispatch {
    ($($ty:ty),* $(,)?) => {
        $(
            impl wayland_client::Dispatch<$ty, ()> for State {
                fn event(
                    _state: &mut Self,
                    _proxy: &$ty,
                    _event: <$ty as wayland_client::Proxy>::Event,
                    _data: &(),
                    _conn: &Connection,
                    _qh: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

empty_dispatch!(
    WlCompositor,
    WlShm,
    WlShmPool,
    WlBuffer,
    WlSurface,
    WlOutput,
    XdgToplevel,
);

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}
