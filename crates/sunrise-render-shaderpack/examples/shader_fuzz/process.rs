//! The fuzzer's child processes (case processes and eval workers), from the side that starts
//! them.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use crate::check::{self, Failure};

/// A child process of this binary: request lines in, reply lines out. Started when first
/// needed and again after a request it could not answer; when it is dropped, its request pipe
/// closes and it is waited for, as at the end of its requests it exits.
pub struct Process {
    exe: PathBuf,
    args: Vec<String>,
    /// The line it is given first, when it starts.
    init: Option<String>,
    running: Option<Running>,
}

pub struct Running {
    pub child: std::process::Child,
    input: std::process::ChildStdin,
    lines: Receiver<String>,
    errors: std::thread::JoinHandle<Vec<u8>>,
}

impl Process {
    pub fn new(exe: &Path, args: &[String], init: Option<String>) -> Self {
        Self {
            exe: exe.to_owned(),
            args: args.to_vec(),
            init,
            running: None,
        }
    }

    pub fn start(&mut self) -> &mut Running {
        let (exe, args, init) = (&self.exe, &self.args, &self.init);
        self.running.get_or_insert_with(|| {
            let mut child = Command::new(exe)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("a child process of the fuzzer starts");
            let mut input = child.stdin.take().expect("piped");
            let output = child.stdout.take().expect("piped");
            let mut errors_in = child.stderr.take().expect("piped");
            let (tx, lines) = channel();
            std::thread::spawn(move || {
                for line in BufReader::new(output).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            // The last 8 KiB the process wrote to stderr, once it has exited (end of the pipe).
            let errors = std::thread::spawn(move || {
                let (mut tail, mut buf) = (Vec::new(), [0u8; 4096]);
                while let Ok(n) = errors_in.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    tail.extend_from_slice(&buf[..n]);
                    let excess = tail.len().saturating_sub(8192);
                    tail.drain(..excess);
                }
                tail
            });
            // A process that can't take it ends, which the first request finds.
            if let Some(init) = init {
                let _ = writeln!(input, "{init}").and_then(|_| input.flush());
            }
            Running {
                child,
                input,
                lines,
                errors,
            }
        })
    }

    /// Sends `line` and waits up to `limit` for the reply.
    pub fn ask(&mut self, line: &str, limit: Duration) -> Result<String, RecvTimeoutError> {
        let running = self.start();
        match writeln!(running.input, "{line}").and_then(|_| running.input.flush()) {
            Ok(()) => running.lines.recv_timeout(limit),
            Err(_) => Err(RecvTimeoutError::Disconnected),
        }
    }

    /// The next reply, within `limit`.
    pub fn next(&mut self, limit: Duration) -> Result<String, RecvTimeoutError> {
        match &self.running {
            Some(running) => running.lines.recv_timeout(limit),
            None => Err(RecvTimeoutError::Disconnected),
        }
    }

    /// The process after a request it did not answer (`reply`, a line that isn't an answer, or
    /// none within `limit`): killed, waited for, and what ended it.
    pub fn fail(
        &mut self,
        reply: &Result<String, RecvTimeoutError>,
        limit: Duration,
        memory_mib: usize,
    ) -> Failure {
        let Some(mut gone) = self.running.take() else {
            return Failure::new("crash", "not started");
        };
        let _ = gone.child.kill();
        let status = gone.child.wait();
        drop(gone.input);
        // All it wrote: the reader ends at the end of the pipe, which the exit closed.
        let errors = gone.errors.join().unwrap_or_default();
        let errors = String::from_utf8_lossy(&errors).into_owned();
        match reply {
            // Only this depends on timing: a case that takes about the limit can land either side.
            Err(RecvTimeoutError::Timeout) => {
                Failure::new("time", format!("over {} s", limit.as_secs()))
            }
            _ if errors.contains("memory allocation of") => {
                Failure::new("memory", format!("over {memory_mib} MiB"))
            }
            _ => Failure::new(
                "crash",
                format!(
                    "{}: {}",
                    status.map_or("?".into(), |s| s.to_string()),
                    check::shape(errors.lines().last().unwrap_or(""))
                ),
            ),
        }
    }

    /// Closes the request pipe and waits for the process, which exits at the end of its requests:
    /// how it ended, and the lines it wrote after its last answer. `None` when it isn't running.
    pub fn close(&mut self) -> Option<Closed> {
        let Running {
            mut child,
            input,
            lines,
            errors,
        } = self.running.take()?;
        drop(input);
        let status = child
            .wait()
            .map_or_else(|e| e.to_string(), |s| s.to_string());
        let _ = errors.join();
        // The reader ends at the end of the pipe, which the exit closed.
        Some(Closed {
            status,
            lines: lines.iter().collect(),
        })
    }
}

/// How a closed process ended (`ExitStatus`'s text), and what it wrote last.
pub struct Closed {
    pub status: String,
    pub lines: Vec<String>,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
