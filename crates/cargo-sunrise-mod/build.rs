//! `SUNRISE_TOOL_CHANNEL` names the release a build is published to; only the rolling `mod-tool`
//! release sets it, and `self-update` follows only that release. The commit a build is made from
//! is `sunrise_native_build::contract::TOOL_COMMIT`.
fn main() {
    println!("cargo:rerun-if-env-changed=SUNRISE_TOOL_CHANNEL");
    let channel = std::env::var("SUNRISE_TOOL_CHANNEL").unwrap_or_default();
    println!("cargo:rustc-env=SUNRISE_TOOL_CHANNEL={channel}");
}
