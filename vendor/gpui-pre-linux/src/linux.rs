// Modified for SparkCommander (Apache-2.0, section 4b): files on the clipboard; dialog
// windows not modal to GNOME Shell.
// Changes are marked "SparkCommander patch".

// SparkCommander patch: files on the clipboard.
mod clipboard_files;
mod dispatcher;
mod display_connection;
mod headless;
mod keyboard;
mod platform;
mod system_notifications;
#[cfg(any(feature = "wayland", feature = "x11"))]
mod text_system;
#[cfg(feature = "wayland")]
mod wayland;
// SparkCommander patch: a window's icon in several sizes.
#[cfg(any(feature = "wayland", feature = "x11"))]
mod window_icon;
#[cfg(feature = "x11")]
mod x11;

#[cfg(any(feature = "wayland", feature = "x11"))]
mod xdg_desktop_portal;

pub use dispatcher::*;
pub(crate) use display_connection::{Backend, DisplayConnection, select_backend};
pub(crate) use headless::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
#[cfg(any(feature = "wayland", feature = "x11"))]
pub(crate) use text_system::*;
#[cfg(feature = "wayland")]
pub(crate) use wayland::*;
#[cfg(feature = "x11")]
pub(crate) use x11::*;

use std::rc::Rc;

use gpui::WindowingModes;

/// Returns the default platform implementation for the current OS.
///
/// A windowed platform connects to the display server the process environment names, or starts
/// headless when it names none, and can switch between those modes later. A headless platform
/// stays headless.
pub fn current_platform(headless: bool) -> Rc<dyn gpui::Platform> {
    if headless || std::env::var_os("ZED_HEADLESS").is_some() {
        linux_platform(WindowingModes::HEADLESS)
    } else {
        linux_platform(WindowingModes::all())
    }
}

/// Returns a platform that may switch among `allowed_modes`.
///
/// It starts windowed in the process's own environment, or headless if that names no allowed
/// display server. Set another initial mode with [`gpui::Application::with_windowing`].
pub fn linux_platform(allowed_modes: WindowingModes) -> Rc<dyn gpui::Platform> {
    Rc::new(LinuxPlatform::new(allowed_modes))
}

/// SparkCommander patch: whether a dialog window is marked modal for the
/// compositor. GNOME Shell attaches a modal one under its parent's title bar
/// and darkens the parent to 70%, which GNOME's own apps' dialogs do not; the
/// parent's input is blocked here all the same (`is_blocked`).
#[cfg(any(feature = "wayland", feature = "x11"))]
pub(crate) fn modal_for_compositor() -> bool {
    !std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').any(|desktop| desktop == "GNOME")
}
