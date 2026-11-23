//! Gives the Windows executable Sunrise's icon and its stack, as sunrise-client's
//! (`packaging/windows/exe.rs`).
#[path = "../../packaging/windows/exe.rs"]
mod windows_exe;

fn main() {
    windows_exe::resources("sunrise-official");
}
