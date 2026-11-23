//! Thin executable for the shared server CLI and supervisor library.
fn main() -> std::process::ExitCode {
    sunrise_platform::prepare_main_stack();
    sunrise_server::main()
}
