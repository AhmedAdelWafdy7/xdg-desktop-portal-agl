#[derive(Debug, Clone, Copy)]
pub struct GlobalInfo {
    pub name: u32,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedShellbackend {
    Agl,
    Xdg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedCapturebackend {
    AglScreenshooter,
    WestonScreenshooter,
    WlrScreencopy,
}

#[derive(Debug, Default)]
pub struct Capabilities {
    pub wl_compositor: Option<GlobalInfo>,
    pub xdg_wm_base: Option<GlobalInfo>,
    pub agl_shell: Option<GlobalInfo>,

    pub wl_shm: Option<GlobalInfo>,
    pub agl_screenshooter: Option<GlobalInfo>,
    pub weston_screenshooter: Option<GlobalInfo>,
    pub zwlr_screencopy_manager: Option<GlobalInfo>,
    pub output: Vec<GlobalInfo>,
}

impl Capabilities {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn first_missing_base(&self) -> Option<&'static str> {
        if self.wl_compositor.is_none() {
            return Some("wl_compositor");
        }
        if self.xdg_wm_base.is_none() {
            return Some("xdg_wm_base");
        }
        if self.agl_shell.is_none() {
            return Some("agl_shell");
        }
        None
    }

    pub fn selected_shell_backend(&self) -> Option<SelectedShellbackend> {
        if self.agl_shell.is_some() {
            Some(SelectedShellbackend::Agl)
        } else if self.xdg_wm_base.is_some() {
            Some(SelectedShellbackend::Xdg)
        } else {
            None
        }
    }

    pub fn selected_capture_backend(&self) -> Option<SelectedCapturebackend> {
        // weston-output-capture is preferred: it is the protocol the AGL compositor advertises
        // and uses for its own reference screenshot client. agl_screenshooter is a legacy
        // fallback, and wlr-screencopy covers wlroots-based compositors.
        if self.weston_screenshooter.is_some() {
            Some(SelectedCapturebackend::WestonScreenshooter)
        } else if self.agl_screenshooter.is_some() {
            Some(SelectedCapturebackend::AglScreenshooter)
        } else if self.zwlr_screencopy_manager.is_some() {
            Some(SelectedCapturebackend::WlrScreencopy)
        } else {
            None
        }
    }
}
