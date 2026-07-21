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

use std::sync::{Arc, Mutex};

use wayland_client::{
    QueueHandle,
    protocol::{wl_compositor::WlCompositor, wl_output, wl_surface::WlSurface},
};

use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase,
};

use super::{Edge, ShellBackend, SurfaceRole};
use crate::error::ShellError;

use crate::protocols::agl_shell::client::agl_shell::{
    AglShell, Edge as AglEdge, Event as AglShellEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AglBindState {
    Pending,
    BoundOk,
    BoundFail,
}

pub struct AglState {
    pub bind_state: Arc<Mutex<AglBindState>>,
}

impl wayland_client::Dispatch<AglShell, ()> for AglState {
    fn event(
        state: &mut Self,
        _proxy: &AglShell,
        event: <AglShell as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            AglShellEvent::BoundOk => {
                *state.bind_state.lock().unwrap() = AglBindState::BoundOk;
            }
            AglShellEvent::BoundFail => {
                *state.bind_state.lock().unwrap() = AglBindState::BoundFail;
            }
            _ => {
                // Handle other events if necessary
            }
        }
    }
}

impl wayland_client::Dispatch<WlSurface, ()> for AglState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: <WlSurface as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // Handle WlSurface events if necessary
    }
}

impl wayland_client::Dispatch<XdgWmBase, ()> for AglState {
    fn event(
        _state: &mut Self,
        proxy: &XdgWmBase,
        event: <XdgWmBase as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_wm_base::Event;
        if let Event::Ping { serial } = event {
            proxy.pong(serial);
            {}
        }
    }
}

impl wayland_client::Dispatch<XdgSurface, ()> for AglState {
    fn event(
        _state: &mut Self,
        proxy: &XdgSurface,
        event: <XdgSurface as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_surface::Event;
        if let Event::Configure { serial } = event {
            proxy.ack_configure(serial);
        }
    }
}

impl wayland_client::Dispatch<XdgToplevel, ()> for AglState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgToplevel,
        _event: <XdgToplevel as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // Handle XdgToplevel events if necessary
    }
}

fn edge_to_agl_edge(edge: Edge) -> AglEdge {
    match edge {
        Edge::Top => AglEdge::Top,
        Edge::Bottom => AglEdge::Bottom,
        Edge::Left => AglEdge::Left,
        Edge::Right => AglEdge::Right,
    }
}

pub struct AglShellsurface {
    pub wl_surface: WlSurface,
    pub xdg_surface: XdgSurface,
    pub xdg_toplevel: Option<XdgToplevel>,
    pub agl_shell: AglShell,
    pub output: wl_output::WlOutput,
    pub role_assigned: bool,
}

impl AglShellsurface {
    pub fn new(
        compositor: &WlCompositor,
        xdg_wm_base: &XdgWmBase,
        agl_shell: AglShell,
        output: wl_output::WlOutput,
        qh: &QueueHandle<AglState>,
    ) -> Self {
        let wl_surface = compositor.create_surface(qh, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface, qh, ());
        Self {
            wl_surface,
            xdg_surface,
            xdg_toplevel: None,
            agl_shell,
            output,
            role_assigned: false,
        }
    }

    pub fn wl_surface(&self) -> &WlSurface {
        &self.wl_surface
    }

    pub fn ready(&self) {
        self.agl_shell.ready();
    }

    pub fn assign_agl_role(
        &mut self,
        role: SurfaceRole,
        qh: &QueueHandle<AglState>,
    ) -> Result<(), ShellError> {
        if self.role_assigned {
            return Err(ShellError::RoleAlreadyAssigned);
        }

        match role {
            SurfaceRole::Toplevel => {
                let xdg_toplevel = self.xdg_surface.get_toplevel(qh, ());
                self.xdg_toplevel = Some(xdg_toplevel);
            }
            SurfaceRole::Fullscreen => {
                let xdg_toplevel = self.xdg_surface.get_toplevel(qh, ());
                xdg_toplevel.set_fullscreen(Some(&self.output));
                self.xdg_toplevel = Some(xdg_toplevel);
            }

            SurfaceRole::Panel(edge) => {
                self.agl_shell
                    .set_panel(&self.wl_surface, &self.output, edge_to_agl_edge(edge));
            }
            SurfaceRole::Background => {
                self.agl_shell
                    .set_background(&self.wl_surface, &self.output);
            }
        }

        self.role_assigned = true;
        Ok(())
    }

    pub fn commit(&self) {
        self.wl_surface.commit();
    }

    pub fn backend(&self) -> ShellBackend {
        ShellBackend::Agl
    }
}
