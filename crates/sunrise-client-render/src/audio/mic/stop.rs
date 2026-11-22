//! A stop signal that wakes the owner of a platform capture stream.

#[cfg(all(
    feature = "audio",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    )
))]
mod platform {
    use std::{
        io::{self, Read},
        net::Shutdown,
        os::{fd::AsRawFd, unix::net::UnixStream},
    };

    pub(in super::super) struct Sender(UnixStream);
    pub(in super::super) struct Receiver(UnixStream);

    pub(in super::super) fn channel() -> io::Result<(Sender, Receiver)> {
        let (tx, rx) = UnixStream::pair()?;
        Ok((Sender(tx), Receiver(rx)))
    }

    impl Sender {
        pub(in super::super) fn signal(&self) {
            let _ = self.0.shutdown(Shutdown::Write);
        }
    }

    impl Receiver {
        pub(in super::super) fn wait(mut self) {
            let _ = self.0.read_exact(&mut [0]);
        }

        pub(in super::super) fn pollfd(&self) -> alsa::poll::pollfd {
            alsa::poll::pollfd {
                fd: self.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            }
        }
    }
}

#[cfg(not(all(
    feature = "audio",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    )
)))]
mod platform {
    use std::{io, sync::mpsc};

    pub(in super::super) struct Sender(mpsc::Sender<()>);
    pub(in super::super) struct Receiver(mpsc::Receiver<()>);

    pub(in super::super) fn channel() -> io::Result<(Sender, Receiver)> {
        let (tx, rx) = mpsc::channel();
        Ok((Sender(tx), Receiver(rx)))
    }

    impl Sender {
        pub(in super::super) fn signal(&self) {
            let _ = self.0.send(());
        }
    }

    impl Receiver {
        pub(in super::super) fn wait(self) {
            let _ = self.0.recv();
        }
    }
}

pub(super) use platform::{Receiver, Sender, channel};
