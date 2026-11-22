//! Embeds Sunrise's Windows and macOS resources in the `sunrise-client` executable.
#[path = "../../packaging/macos/exe.rs"]
mod macos_exe;
#[path = "../../packaging/windows/exe.rs"]
mod windows_exe;

fn main() {
    windows_exe::resources("sunrise-client");
    macos_exe::resources("sunrise-client");
}
