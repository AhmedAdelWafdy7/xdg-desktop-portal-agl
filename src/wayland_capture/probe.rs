// Copyright 2026 Ahmed Wafdy <ahmedadelwafdy782@gmail.com>
//
// This file is part of xdg-desktop-portal-agl.
//
// xdg-desktop-portal-agl is free software: you can redistribute it and/or
// modify it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 2 of the License, or
// (at your option) any later version.
//
// xdg-desktop-portal-agl is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General
// Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// xdg-desktop-portal-agl. If not, see <https://www.gnu.org/licenses/>.

//! Connects to the Wayland compositor, enumerates globals, and binds the objects needed
//! to perform a screenshot. The result is a [`WaylandConnection`] holding the compositor
//! connection together with the capture managers (weston-output-capture, wlr-screencopy or
//! agl-screenshooter) that the compositor happens to advertise.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::WlCallback,
        wl_output::{self, Transform, WlOutput},
        wl_registry::{self, WlRegistry},
        wl_shm::WlShm,
    },
};

use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::capture::types::CaptureError;
use crate::capture::{capture_timeout, dispatch_until};
use crate::protocols::weston_output_capture::client::weston_capture_v1::WestonCaptureV1;
use crate::registry::{Capabilities, GlobalInfo};

/// A single output (screen) discovered on the compositor.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    /// Registry global name, which is what identifies the output across hotplug: indices shift
    /// when an output is unplugged, this does not.
    pub global_name: u32,
    pub wl_output: WlOutput,
    pub name: Option<String>,
    /// Current mode size, in physical (pre-transform) pixels — that is what `wl_output.mode`
    /// reports. On a rotated output this is *not* the logical size; see [`OutputInfo::transform`].
    pub width: i32,
    pub height: i32,
    /// Rotation/flip the compositor applies between the framebuffer and the logical desktop.
    pub transform: Transform,
}

impl OutputInfo {
    fn new(global_name: u32, wl_output: WlOutput) -> Self {
        Self {
            global_name,
            wl_output,
            name: None,
            width: 0,
            height: 0,
            transform: Transform::Normal,
        }
    }

    /// Whether the transform swaps the width and height axes (90 / 270 degrees, with or without
    /// a flip). Portrait instrument clusters and centre stacks are usually driven this way.
    pub fn swaps_axes(&self) -> bool {
        transform_swaps_axes(self.transform)
    }
}

/// Whether `transform` exchanges the width and height axes.
///
/// The four rotations that do are 90 and 270, each with and without a flip; a flip on its own
/// mirrors within the same axes. Split out from [`OutputInfo`] so it can be tested without a
/// live compositor to bind a `wl_output` against.
pub fn transform_swaps_axes(transform: Transform) -> bool {
    matches!(
        transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    )
}

/// Selects which output to capture.
#[derive(Debug, Clone)]
pub enum OutputSelector {
    /// The first output reported by the compositor.
    First,
    /// The output at the given index in discovery order.
    Index(usize),
    /// The output whose name matches (e.g. "HDMI-A-1").
    Name(String),
}

/// The output list plus the queue that keeps it current.
///
/// `connect()` used to snapshot output geometry on an event queue that died as soon as it
/// returned, so a cached connection served whatever the outputs looked like at first use —
/// a mode change or a hotplug after that was never seen, and the agl backend would go on
/// sizing its buffer from the stale numbers. Keeping the queue alive lets [`refresh`] pull
/// the current state before each capture.
struct OutputRegistry {
    queue: EventQueue<ProbeState>,
    state: ProbeState,
}

impl OutputRegistry {
    /// Re-read output state from the compositor, bounded by the capture timeout.
    ///
    /// A plain `roundtrip` would block forever against a compositor that stops answering, which
    /// is the same unbounded wait `dispatch_until` exists to avoid — the portal runs captures on
    /// `spawn_blocking`, so a parked thread leaks per request.
    fn refresh(&mut self, conn: &Connection) -> Result<(), CaptureError> {
        let qh = self.queue.handle();
        self.state.sync_done = false;
        let _sync = conn.display().sync(&qh, ());
        let deadline = Instant::now() + capture_timeout();
        dispatch_until(conn, &mut self.queue, &mut self.state, deadline, |s| {
            s.sync_done
        })
    }
}

/// An open connection to the compositor with the globals required for screenshotting bound.
#[derive(Clone)]
pub struct WaylandConnection {
    pub conn: Connection,
    pub registry: WlRegistry,
    pub shm: WlShm,
    outputs: Arc<Mutex<OutputRegistry>>,
    pub weston_capture: Option<WestonCaptureV1>,
    pub wlr_screencopy: Option<ZwlrScreencopyManagerV1>,
    pub capabilities: Capabilities,
}

impl WaylandConnection {
    /// Pull current output geometry and membership from the compositor.
    ///
    /// Call this before reading outputs on a connection that has been held across requests;
    /// `capture_on` does it for you.
    pub fn refresh_outputs(&self) -> Result<(), CaptureError> {
        self.outputs.lock().unwrap().refresh(&self.conn)
    }

    /// Snapshot of the outputs currently known, in discovery order.
    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.outputs.lock().unwrap().state.outputs.clone()
    }

    /// Resolve an [`OutputSelector`] against the discovered outputs.
    pub fn select_output(&self, selector: &OutputSelector) -> Option<OutputInfo> {
        let registry = self.outputs.lock().unwrap();
        let outputs = &registry.state.outputs;
        match selector {
            OutputSelector::First => outputs.first().cloned(),
            OutputSelector::Index(i) => outputs.get(*i).cloned(),
            OutputSelector::Name(n) => outputs
                .iter()
                .find(|o| o.name.as_deref() == Some(n.as_str()))
                .cloned(),
        }
    }
}

// State used while enumerating and binding globals. Output metadata is collected here as the
// compositor replies to the initial bind with geometry/name/mode events, and kept current
// afterwards by the same queue.
#[derive(Default)]
struct ProbeState {
    outputs: Vec<OutputInfo>,
    // Set when a wl_display.sync callback fires, which bounds a refresh.
    sync_done: bool,
}

/// Connect to the compositor named by `WAYLAND_DISPLAY` and discover screenshot capabilities.
pub fn connect() -> Result<WaylandConnection, CaptureError> {
    let conn = Connection::connect_to_env()
        .map_err(|e| CaptureError::WaylandError(format!("connect: {e}")))?;

    let (globals, mut event_queue) = registry_queue_init::<ProbeState>(&conn)
        .map_err(|e| CaptureError::WaylandError(format!("registry init: {e}")))?;
    let qh = event_queue.handle();
    let registry = globals.registry();
    let contents = globals.contents().clone_list();

    let mut caps = Capabilities::new();
    let mut state = ProbeState::default();

    let mut shm: Option<WlShm> = None;
    let mut weston_capture: Option<WestonCaptureV1> = None;
    let mut wlr_screencopy: Option<ZwlrScreencopyManagerV1> = None;

    for g in &contents {
        let info = GlobalInfo {
            name: g.name,
            version: g.version,
        };
        match g.interface.as_str() {
            "wl_shm" => {
                caps.wl_shm = Some(info);
                shm = Some(registry.bind::<WlShm, _, _>(g.name, g.version.min(1), &qh, ()));
            }
            "weston_capture_v1" => {
                caps.weston_screenshooter = Some(info);
                weston_capture =
                    Some(registry.bind::<WestonCaptureV1, _, _>(g.name, g.version.min(1), &qh, ()));
            }
            "zwlr_screencopy_manager_v1" => {
                caps.zwlr_screencopy_manager = Some(info);
                wlr_screencopy = Some(registry.bind::<ZwlrScreencopyManagerV1, _, _>(
                    g.name,
                    g.version.min(1),
                    &qh,
                    (),
                ));
            }
            // Recorded but not bound: the agl backend rebinds by global name onto its own
            // event queue, because `done` is emitted on the global itself.
            "agl_screenshooter" => caps.agl_screenshooter = Some(info),
            "wl_output" => {
                let wl_output =
                    registry.bind::<WlOutput, _, _>(g.name, g.version.min(4), &qh, g.name);
                state.outputs.push(OutputInfo::new(g.name, wl_output));
            }
            _ => {}
        }
    }

    // Drive the initial burst of wl_output (and wl_shm) events so output geometry/names arrive.
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| CaptureError::WaylandError(format!("roundtrip: {e}")))?;

    let shm =
        shm.ok_or_else(|| CaptureError::CaptureFailed("compositor exposes no wl_shm".into()))?;

    Ok(WaylandConnection {
        conn,
        registry: registry.clone(),
        shm,
        outputs: Arc::new(Mutex::new(OutputRegistry {
            queue: event_queue,
            state,
        })),
        weston_capture,
        wlr_screencopy,
        capabilities: caps,
    })
}

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for ProbeState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // Capture globals are enumerated once up front — a compositor withdrawing one mid-run
        // surfaces as a protocol error on the next capture, which invalidates the connection.
        // Outputs are different: they come and go on a running IVI system, so they are tracked.
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_output" => {
                if state.outputs.iter().any(|o| o.global_name == name) {
                    return;
                }
                let wl_output = registry.bind::<WlOutput, _, _>(name, version.min(4), qh, name);
                state.outputs.push(OutputInfo::new(name, wl_output));
            }
            wl_registry::Event::GlobalRemove { name } => {
                let Some(pos) = state.outputs.iter().position(|o| o.global_name == name) else {
                    return;
                };
                let removed = state.outputs.remove(pos);
                // `release` is the v3+ destructor; on older outputs the proxy just goes away.
                if removed.wl_output.version() >= 3 {
                    removed.wl_output.release();
                }
            }
            _ => {}
        }
    }
}

impl wayland_client::Dispatch<WlOutput, u32> for ProbeState {
    fn event(
        state: &mut Self,
        _proxy: &WlOutput,
        event: wl_output::Event,
        global_name: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(out) = state
            .outputs
            .iter_mut()
            .find(|o| o.global_name == *global_name)
        else {
            return;
        };
        match event {
            wl_output::Event::Mode {
                flags: WEnum::Value(m),
                width,
                height,
                ..
            } if m.contains(wl_output::Mode::Current) => {
                out.width = width;
                out.height = height;
            }
            wl_output::Event::Geometry {
                transform: WEnum::Value(t),
                ..
            } => {
                out.transform = t;
            }
            wl_output::Event::Name { name } => {
                out.name = Some(name);
            }
            _ => {}
        }
    }
}

impl wayland_client::Dispatch<WlCallback, ()> for ProbeState {
    fn event(
        state: &mut Self,
        _proxy: &WlCallback,
        _event: wayland_client::protocol::wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.sync_done = true;
    }
}

crate::capture::impl_noop_dispatch!(ProbeState, WlShm, WestonCaptureV1, ZwlrScreencopyManagerV1);
