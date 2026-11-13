use super::*;
#[cfg(unix)]
use std::{io::BufRead, process::Stdio};
use std::{io::Write, process::Command};

fn directory(path: &Path) -> Dir {
    std::fs::create_dir_all(path).unwrap();
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
}

#[cfg(unix)]
fn released(registry: &std::sync::Arc<Registry>, tree: &Dir) {
    // Another parallel fork can briefly retain an untouched CLOEXEC copy until its exec.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        match registry.lock(tree, LockMode::Exclusive) {
            Ok(claim) => {
                drop(claim);
                return;
            }
            Err(Error::Busy) if std::time::Instant::now() < deadline => std::thread::yield_now(),
            _ => panic!("claim remained held after failed exec or child exit"),
        }
    }
}

#[cfg(unix)]
#[test]
fn child_keeps_gate_and_tree_after_creator_closes_its_copies() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let trees = [directory(&root.path().join("tree"))];
    if let Ok(mask) = std::env::var("HORIZON_TEST_CLOSED_STDIO") {
        let mask: u8 = mask.parse().unwrap();
        // SAFETY: only the isolated regression subprocess closes its own stdio.
        for fd in 0..3 {
            if mask & (1 << fd) != 0 {
                unsafe {
                    libc::close(fd);
                }
            }
        }
    }
    let claims = ChildLeases::acquire(&data, &trees).unwrap();
    use std::os::fd::AsRawFd;
    for file in claims.files() {
        // SAFETY: file owns this descriptor in the current process.
        assert_ne!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process::leases::tests::bridge_actor",
            "--nocapture",
        ])
        .env("HORIZON_LEASE_BRIDGE_ACTOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var_os("HORIZON_TEST_STDIO_CLOEXEC").is_some() {
        use std::os::unix::process::CommandExt;
        // Model the post-dup2(fd, fd) state for every slot independently of
        // Command's pipe allocation order. Only fcntl runs after fork.
        unsafe {
            command.pre_exec(|| {
                for fd in 0..3 {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    let mut child = claims.spawn(command).unwrap();
    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("BRIDGE-READY") {
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "actor exited before readiness"
        );
    }
    let gate = File::options()
        .read(true)
        .write(true)
        .open(data.join("run-leases/gate"))
        .unwrap();
    assert!(matches!(
        gate.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    assert!(matches!(
        registry.lock(&trees[0], LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(
        registry
            .lock(
                &directory(&root.path().join("unrelated")),
                LockMode::Exclusive,
            )
            .unwrap(),
    );
    child.stdin.take().unwrap().write_all(b"exit\n").unwrap();
    assert!(child.wait().unwrap().success());
    use std::io::Read;
    let mut remaining = String::new();
    output.read_to_string(&mut remaining).unwrap();
    assert!(remaining.contains("BRIDGE-EXIT"));
    let mut errors = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    assert!(errors.contains("BRIDGE-STDERR"));
    released(&registry, &trees[0]);
    drop(registry);
    gate.try_lock().unwrap();
    gate.unlock().unwrap();
}

#[cfg(windows)]
mod windows_tests;

#[test]
fn bridge_actor() {
    if std::env::var_os("HORIZON_LEASE_BRIDGE_ACTOR").is_none() {
        return;
    }
    println!("BRIDGE-READY");
    std::io::stdout().flush().unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    assert_eq!(input, "exit\n");
    eprintln!("BRIDGE-STDERR");
    println!("BRIDGE-EXIT");
}

#[cfg(unix)]
#[test]
fn closed_stdin_does_not_lose_tree() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process::leases::tests::child_keeps_gate_and_tree_after_creator_closes_its_copies",
            "--nocapture",
        ])
        .env("HORIZON_TEST_CLOSED_STDIO", "1")
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn all_closed_stdio_masks_preserve_configured_pipes() {
    for mask in 1..=7 {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::leases::tests::child_keeps_gate_and_tree_after_creator_closes_its_copies",
                "--nocapture",
            ])
            .env("HORIZON_TEST_CLOSED_STDIO", mask.to_string())
            .status()
            .unwrap();
        assert!(status.success(), "closed stdio mask {mask}");
    }
}

#[cfg(unix)]
#[test]
fn configured_stdio_cloexec_is_cleared_after_setup() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process::leases::tests::child_keeps_gate_and_tree_after_creator_closes_its_copies",
            "--nocapture",
        ])
        .env("HORIZON_TEST_STDIO_CLOEXEC", "1")
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn creator_flags_remain_cloexec_until_child_callback() {
    use std::os::unix::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let trees = [directory(&root.path().join("tree"))];
    let claims = ChildLeases::acquire(&root.path().join("data"), &trees).unwrap();
    // SAFETY: sysconf/fcntl only query this process. Snapshot all slots, including vacant
    // ones later used for transfer duplicates; ignore pre-existing inherited non-CLOEXEC fds.
    let maximum = i32::try_from(unsafe { libc::sysconf(libc::_SC_OPEN_MAX) }).unwrap();
    let flags: Vec<_> = (3..maximum)
        .map(|fd| (fd, unsafe { libc::fcntl(fd, libc::F_GETFD) }))
        .collect();
    let mut command = Command::new("/bin/true");
    // This callback precedes the transfer callback and sees the fork's untouched flags.
    unsafe {
        command.pre_exec(move || {
            for &(fd, before) in &flags {
                let after = libc::fcntl(fd, libc::F_GETFD);
                if after >= 0 && before != 0 && after & libc::FD_CLOEXEC == 0 {
                    return Err(io::Error::from_raw_os_error(libc::EINVAL));
                }
            }
            Ok(())
        });
    }
    let child = claims.spawn(command);
    assert!(
        child.is_ok(),
        "creator cleared CLOEXEC before its child's callback"
    );
    assert!(child.unwrap().wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn failed_exec_leaves_no_tree_or_gate_owner() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let trees = [directory(&root.path().join("tree"))];
    let claims = ChildLeases::acquire(&data, &trees).unwrap();
    assert!(claims
        .spawn(Command::new(root.path().join("missing-program")))
        .is_err());
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    released(&registry, &trees[0]);
    drop(registry);
    let gate = File::options()
        .read(true)
        .write(true)
        .open(data.join("run-leases/gate"))
        .unwrap();
    gate.try_lock().unwrap();
    gate.unlock().unwrap();
}
