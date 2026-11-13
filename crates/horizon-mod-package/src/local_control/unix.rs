use super::{Marker, check_deadline, pause};
use crate::runtime_leases::open_directory;
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::{
    io,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Instant,
};
pub type Stream = UnixStream;

pub struct Endpoint {
    parent: Dir,
    path: PathBuf,
    listener: Option<UnixListener>,
}
impl Endpoint {
    pub fn bind(marker: &Marker) -> io::Result<Self> {
        let parent = marker
            .directory
            .parent()
            .ok_or(io::ErrorKind::InvalidInput)?;
        let parent = open_directory(parent).map_err(|_| io::ErrorKind::InvalidInput)?;
        check_directory(&parent, false)?;
        for ancestor in marker.directory.ancestors().skip(2) {
            let directory = open_directory(ancestor).map_err(|_| io::ErrorKind::InvalidInput)?;
            check_directory(&directory, false)?;
        }
        let name = marker
            .directory
            .file_name()
            .ok_or(io::ErrorKind::InvalidInput)?;
        rustix::fs::mkdirat(&parent, name, rustix::fs::Mode::RWXU)?;
        let mut endpoint = Self {
            parent,
            path: marker.directory.clone(),
            listener: None,
        };
        let directory = endpoint.parent.open_dir_nofollow(name)?;
        check_directory(&directory, true)?;
        let listener = UnixListener::bind(endpoint.path.join("control"))?;
        listener.set_nonblocking(true)?;
        endpoint.listener = Some(listener);
        Ok(endpoint)
    }
    /// Consumes the sole listener: no second client can ever be accepted.
    pub fn accept(&mut self, deadline: Instant) -> io::Result<Stream> {
        let listener = self.listener.take().ok_or(io::ErrorKind::NotConnected)?;
        loop {
            check_deadline(deadline)?;
            match listener.accept() {
                Ok((stream, _)) => {
                    peer(&stream)?;
                    // A concurrent fork can keep the listening descriptor open
                    // until exec. Remove its address before returning so even
                    // that inherited listener cannot admit another connection.
                    let directory = self.parent.open_dir_nofollow(
                        self.path
                            .file_name()
                            .expect("bound endpoint has a directory name"),
                    )?;
                    directory.remove_file("control")?;
                    return Ok(stream);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => pause(deadline)?,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}
pub fn connect(marker: &Marker, deadline: Instant) -> io::Result<Stream> {
    loop {
        check_deadline(deadline)?;
        let directory = match open_directory(&marker.directory) {
            Ok(directory) => directory,
            Err(crate::runtime_leases::Error::Io(error))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                pause(deadline)?;
                continue;
            }
            Err(_) => return Err(io::ErrorKind::PermissionDenied.into()),
        };
        check_directory(&directory, true)?;
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
        let address = socket2::SockAddr::unix(marker.directory.join("control"))?;
        // AF_UNIX backlog exhaustion can report EAGAIN without a pending connection.
        let connected = socket
            .connect_timeout(&address, deadline.saturating_duration_since(Instant::now()))
            .and_then(|()| socket.peer_addr());
        match connected {
            Ok(_) => {
                let stream = Stream::from(std::os::fd::OwnedFd::from(socket));
                peer(&stream)?;
                return Ok(stream);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::NotConnected
                ) =>
            {
                pause(deadline)?
            }
            Err(error) => return Err(error),
        }
    }
}
fn check_directory(directory: &Dir, private: bool) -> io::Result<()> {
    let metadata = rustix::fs::fstat(directory)?;
    if (metadata.st_uid != unsafe { libc::geteuid() } && (private || metadata.st_uid != 0))
        || (metadata.st_mode & 0o022 != 0 && (private || metadata.st_mode & 0o1000 == 0))
        || (private && metadata.st_mode & 0o777 != 0o700)
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}
fn peer(stream: &Stream) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let uid = rustix::net::sockopt::socket_peercred(stream)?.uid.as_raw();
    #[cfg(target_os = "macos")]
    let uid = {
        use std::os::fd::AsRawFd;
        let (mut uid, mut gid) = (0, 0);
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        uid
    };
    #[cfg(test)]
    let uid = TEST_PEER_UID.with(|source| source.get().unwrap_or(uid));
    check_peer_uid(uid)
}
fn check_peer_uid(uid: u32) -> io::Result<()> {
    if uid != unsafe { libc::geteuid() } {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}
#[cfg(test)]
std::thread_local! {
    // Per-thread override keeps parallel real-credential tests isolated.
    static TEST_PEER_UID: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}
#[test]
fn foreign_peer_uid_is_refused_on_connect_and_accept() {
    for server in [false, true] {
        let (_root, marker) = super::tests::marker();
        let mut endpoint = Endpoint::bind(&marker).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let _client = server.then(|| connect(&marker, deadline).unwrap());
        TEST_PEER_UID.set(Some(unsafe { libc::geteuid() } ^ 1));
        let result = if server {
            endpoint.accept(deadline)
        } else {
            connect(&marker, deadline)
        };
        TEST_PEER_UID.set(None);
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    }
}
#[test]
fn consumed_listener_refuses_connections_even_while_an_inherited_copy_is_open() {
    let (_root, marker) = super::tests::marker();
    let mut endpoint = Endpoint::bind(&marker).unwrap();
    // A concurrent fork retains this reference until exec, even with CLOEXEC.
    // Duplicate it explicitly so the regression never depends on scheduling.
    let inherited = endpoint.listener.as_ref().unwrap().try_clone().unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let _client = connect(&marker, deadline).unwrap();
    let _accepted = endpoint.accept(deadline).unwrap();
    assert!(
        UnixStream::connect(marker.directory.join("control")).is_err(),
        "a consumed endpoint still accepts connections through an inherited listener"
    );
    drop(inherited);
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.listener.take();
        if let Some(name) = self.path.file_name() {
            if let Ok(directory) = self.parent.open_dir_nofollow(name) {
                let _ = directory.remove_file("control");
            }
            let _ = self.parent.remove_dir(name);
        }
    }
}
