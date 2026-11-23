/// At the reporter's measured ceiling of about 1.8 KB per frame, 128 MiB
/// covers about 70k plain frames, including Java's deepest reported 59k case.
#[cfg(not(all(target_os = "android", target_pointer_width = "32")))]
pub const GUEST_STACK_BYTES: usize = 128 * 1024 * 1024;
/// 32-bit Android runs wasm on Pulley, Wasmtime's interpreter, whose stack is this much heap
/// reserved per group (`Config::max_wasm_stack`), not the thread's: 128 MiB a group would take
/// the address space the threads' reservations took before (#590). 16 MiB covers about 9k plain
/// frames at the same 1.8 KB.
#[cfg(all(target_os = "android", target_pointer_width = "32"))]
pub const GUEST_STACK_BYTES: usize = 16 * 1024 * 1024;
/// Room for Rust host frames below the guest and synchronous host imports above it.
pub const HOST_HEADROOM: usize = 8 * 1024 * 1024;
