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

pub mod agl;
pub mod xdg;

use crate::error::ShellError;
use wayland_client::QueueHandle;

// which edge an AGL panel is attached to, if any. Mirroring the AGL shell's edge enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceRole {
    Toplevel,
    Fullscreen,
    Panel(Edge),
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellBackend {
    Xdg,
    Agl,
}

// Shell Abstraction Layer, to unify the handling of different shell protocols (xdg-shell and AGL shell).
pub trait ShellSurface {
    fn assign_role(
        &mut self,
        role: SurfaceRole,
        qh: &QueueHandle<xdg::XdgState>,
    ) -> Result<(), ShellError>;

    // commit surface state to the compositor, applying any pending role changes or state updates.
    fn commit(&self);

    // which backend produced this surface
    fn backend(&self) -> ShellBackend;
}
