//! Embeds Sunrise's Windows and macOS resources in the `sunrise-official` executable.
#[path = "../../packaging/macos/exe.rs"]
mod macos_exe;
#[path = "../../packaging/windows/exe.rs"]
mod windows_exe;

fn main() {
    windows_exe::resources("sunrise-official");
    macos_exe::resources("sunrise-official");
}
