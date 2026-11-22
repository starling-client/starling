//! Safe wrappers for the native input APIs the renderer cannot call directly.

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd"
))]
pub mod alsa;
#[cfg(target_os = "macos")]
pub mod macos;
