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

use wayland_client::{
    QueueHandle,
    protocol::{wl_compositor::WlCompositor, wl_surface::WlSurface},
};

use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase,
};

use super::{ShellBackend, ShellSurface, SurfaceRole};
use crate::error::ShellError;

pub struct XdgState;

impl wayland_client::Dispatch<XdgWmBase, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgWmBase,
        _event: wayland_protocols::xdg::shell::client::xdg_wm_base::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_wm_base::Event;
        if let Event::Ping { serial } = _event {
            _proxy.pong(serial);
        }
    }
}

impl wayland_client::Dispatch<XdgSurface, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgSurface,
        _event: wayland_protocols::xdg::shell::client::xdg_surface::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_surface::Event;
        if let Event::Configure { serial } = _event {
            _proxy.ack_configure(serial);
        }
    }
}

impl wayland_client::Dispatch<XdgToplevel, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgToplevel,
        _event: wayland_protocols::xdg::shell::client::xdg_toplevel::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events we care about
    }
}

impl wayland_client::Dispatch<WlSurface, ()> for XdgState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: wayland_client::protocol::wl_surface::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // No events we care about
    }
}

pub struct XdgShellSurface {
    wl_surface: WlSurface,
    #[allow(dead_code)]
    xdg_surface: XdgSurface,
    #[allow(dead_code)]
    xdg_toplevel: Option<XdgToplevel>,
    role_assigned: bool,
}

impl XdgShellSurface {
    pub fn new<D>(
        compositor: &WlCompositor,
        xdg_wm_base: &XdgWmBase,
        queue_handle: &QueueHandle<XdgState>,
    ) -> Self
    where
        D: wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()>
            + wayland_client::Dispatch<XdgSurface, ()>
            + 'static,
    {
        let wl_surface = compositor.create_surface(queue_handle, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface, queue_handle, ());
        Self {
            wl_surface,
            xdg_surface,
            xdg_toplevel: None,
            role_assigned: false,
        }
    }

    pub fn new_with_state<D>(
        compositor: &WlCompositor,
        xdg_wm_base: &XdgWmBase,
        queue_handle: &QueueHandle<D>,
    ) -> Self
    where
        D: wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()>
            + wayland_client::Dispatch<XdgSurface, ()>
            + 'static,
    {
        let wl_surface = compositor.create_surface(queue_handle, ());
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface, queue_handle, ());
        Self {
            wl_surface,
            xdg_surface,
            xdg_toplevel: None,
            role_assigned: false,
        }
    }
}

impl ShellSurface for XdgShellSurface {
    fn assign_role(
        &mut self,
        role: SurfaceRole,
        qh: &QueueHandle<XdgState>,
    ) -> Result<(), ShellError> {
        if self.role_assigned {
            return Err(ShellError::RoleAlreadyAssigned);
        }
        match role {
            SurfaceRole::Toplevel => {
                let toplevel = self.xdg_surface.get_toplevel(qh, ());
                self.xdg_toplevel = Some(toplevel);
            }

            SurfaceRole::Fullscreen => {
                let toplevel = self.xdg_surface.get_toplevel(qh, ());
                toplevel.set_fullscreen(None);
                self.xdg_toplevel = Some(toplevel);
            }

            SurfaceRole::Panel(_) | SurfaceRole::Background => {
                return Err(ShellError::IncompatibleRole(role));
            }
        }
        self.role_assigned = true;
        Ok(())
    }
    fn commit(&self) {
        self.wl_surface.commit();
    }

    fn backend(&self) -> ShellBackend {
        ShellBackend::Xdg
    }
}

impl XdgShellSurface {
    pub fn wl_surface(&self) -> &WlSurface {
        &self.wl_surface
    }
}
