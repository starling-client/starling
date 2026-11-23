use std::io::{self, Write};
use std::process::ExitCode;

use sunrise_perf::cli::{CliError, Env, run, usage};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let result = run(&args, &Env::from_process(), &mut out);
    let _ = out.flush();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            eprintln!("sunrise-perf: {message}\n{}", usage());
            ExitCode::from(2)
        }
        Err(CliError::Failed(message)) => {
            eprintln!("sunrise-perf: {message}");
            ExitCode::FAILURE
        }
    }
}
