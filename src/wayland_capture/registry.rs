#[derive(Debug, Clone, Copy)]
pub struct GlobalInfo {
    pub name: u32,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedCapturebackend {
    AglScreenshooter,
    WestonScreenshooter,
    WlrScreencopy,
}

/// Globals relevant to screen capture, as advertised by the compositor.
///
/// A screenshot portal is a surfaceless client: it needs `wl_shm`, a capture protocol, and a
/// `wl_output`, and never creates a surface. Shell globals are deliberately absent — requiring
/// one would make the portal refuse to run on compositors that capture perfectly well.
#[derive(Debug, Default, Clone, Copy)]
pub struct Capabilities {
    pub wl_shm: Option<GlobalInfo>,
    pub agl_screenshooter: Option<GlobalInfo>,
    pub weston_screenshooter: Option<GlobalInfo>,
    pub zwlr_screencopy_manager: Option<GlobalInfo>,
}

impl Capabilities {
    pub fn new() -> Self {
        Self::default()
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
