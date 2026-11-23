//! Embeds Sunrise's icon (`packaging/icons/sunrise.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
fn main() {
    println!("cargo:rerun-if-env-changed=SUNRISE_LAUNCHER_REPOSITORY");
    println!("cargo:rustc-check-cfg=cfg(sunrise_fork_launcher)");
    let repository =
        std::env::var("SUNRISE_LAUNCHER_REPOSITORY").unwrap_or_else(|_| "arismoko/horizon".into());
    match repository.as_str() {
        "arismoko/horizon" => println!("cargo:rustc-cfg=sunrise_fork_launcher"),
        "horizon-rust/horizon" => {}
        _ => panic!("unsupported launcher repository: {repository}"),
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Resolve imports before main from System32, never a nearby injector DLL (#13108).
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let icon = if repository == "arismoko/horizon" {
        "fork/launcher.ico"
    } else {
        "sunrise.ico"
    };
    let ico = format!("{dir}/../packaging/icons/{icon}");
    println!("cargo:rerun-if-changed={ico}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if repository == "arismoko/horizon" {
            resource.set("FileDescription", "Sunrise Fork Launcher (arismoko)");
            resource.set("ProductName", "Sunrise Fork Launcher (arismoko)");
            resource.set("InternalName", "sunrise-fork-launcher");
            resource.set("OriginalFilename", "sunrise-fork-launcher.exe");
        }
        if let Err(e) = resource.compile() {
            panic!("embedding {ico}: {e}");
        }
    }
}
