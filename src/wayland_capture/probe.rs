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

use wayland_client::{
    Connection, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_output::{self, WlOutput},
        wl_registry::WlRegistry,
        wl_shm::WlShm,
    },
};

use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::capture::types::CaptureError;
use crate::protocols::agl_screenshooter::client::agl_screenshooter::AglScreenshooter;
use crate::protocols::weston_output_capture::client::weston_capture_v1::WestonCaptureV1;
use crate::registry::{Capabilities, GlobalInfo};

/// A single output (screen) discovered on the compositor.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub wl_output: WlOutput,
    pub name: Option<String>,
    pub width: i32,
    pub height: i32,
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

/// An open connection to the compositor with the globals required for screenshotting bound.
pub struct WaylandConnection {
    pub conn: Connection,
    pub shm: WlShm,
    pub outputs: Vec<OutputInfo>,
    pub weston_capture: Option<WestonCaptureV1>,
    pub wlr_screencopy: Option<ZwlrScreencopyManagerV1>,
    pub agl_screenshooter: Option<AglScreenshooter>,
    pub capabilities: Capabilities,
}

impl WaylandConnection {
    /// Resolve an [`OutputSelector`] against the discovered outputs.
    pub fn select_output(&self, selector: &OutputSelector) -> Option<&OutputInfo> {
        match selector {
            OutputSelector::First => self.outputs.first(),
            OutputSelector::Index(i) => self.outputs.get(*i),
            OutputSelector::Name(n) => self
                .outputs
                .iter()
                .find(|o| o.name.as_deref() == Some(n.as_str())),
        }
    }
}

// State used while enumerating and binding globals. Output metadata is collected here as the
// compositor replies to the initial bind with geometry/name/mode events.
#[derive(Default)]
struct ProbeState {
    outputs: Vec<OutputInfo>,
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
    let mut agl_screenshooter: Option<AglScreenshooter> = None;

    for g in &contents {
        let info = GlobalInfo {
            name: g.name,
            version: g.version,
        };
        match g.interface.as_str() {
            "wl_compositor" => caps.wl_compositor = Some(info),
            "xdg_wm_base" => caps.xdg_wm_base = Some(info),
            "agl_shell" => caps.agl_shell = Some(info),
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
                    g.version.min(3),
                    &qh,
                    (),
                ));
            }
            "agl_screenshooter" => {
                caps.agl_screenshooter = Some(info);
                agl_screenshooter = Some(registry.bind::<AglScreenshooter, _, _>(
                    g.name,
                    g.version.min(1),
                    &qh,
                    (),
                ));
            }
            "wl_output" => {
                caps.output.push(info);
                let idx = state.outputs.len();
                let wl_output = registry.bind::<WlOutput, _, _>(g.name, g.version.min(4), &qh, idx);
                state.outputs.push(OutputInfo {
                    wl_output,
                    name: None,
                    width: 0,
                    height: 0,
                });
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
        shm,
        outputs: state.outputs,
        weston_capture,
        wlr_screencopy,
        agl_screenshooter,
        capabilities: caps,
    })
}

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Globals are enumerated up front via GlobalList; runtime add/remove is ignored.
    }
}

impl wayland_client::Dispatch<WlOutput, usize> for ProbeState {
    fn event(
        state: &mut Self,
        _proxy: &WlOutput,
        event: wl_output::Event,
        data: &usize,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(out) = state.outputs.get_mut(*data) else {
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
            wl_output::Event::Name { name } => {
                out.name = Some(name);
            }
            _ => {}
        }
    }
}

impl wayland_client::Dispatch<WlShm, ()> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wayland_client::protocol::wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<WestonCaptureV1, ()> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &WestonCaptureV1,
        _event: <WestonCaptureV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<ZwlrScreencopyManagerV1, ()> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrScreencopyManagerV1,
        _event: <ZwlrScreencopyManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<AglScreenshooter, ()> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &AglScreenshooter,
        _event: <AglScreenshooter as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}
