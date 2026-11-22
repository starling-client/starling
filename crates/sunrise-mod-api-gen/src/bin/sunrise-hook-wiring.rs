//! Run the catalog's read-only engine wiring check on a checkout.
use std::{path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(root) = args.next().map(PathBuf::from) else {
        eprintln!("usage: sunrise-hook-wiring CHECKOUT TARGET_CFG...");
        return ExitCode::FAILURE;
    };
    let Ok(configs) = args
        .map(|arg| arg.into_string())
        .collect::<Result<Vec<_>, _>>()
    else {
        eprintln!("target cfgs must be UTF-8");
        return ExitCode::FAILURE;
    };
    if let Err(error) = sunrise_mod_api_gen::check_hook_wiring(&root, &configs) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    println!("Native hook wiring check passed.");
    ExitCode::SUCCESS
}
