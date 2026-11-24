//! What the system and the desktop give: their look, disks, menus, clipboard,
//! shells and administrator rights.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod adwaita;
pub mod clipboard;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod desktop;
pub mod devices;
pub mod elevate;
#[cfg(target_os = "macos")]
pub mod finder;
pub mod properties;
#[cfg(windows)]
pub mod shell_icon;
#[cfg(windows)]
pub mod shell_menu;
pub mod shells;
pub mod tags;
pub mod theme;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod toolkit_style;
