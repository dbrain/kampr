use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::Duration;

/// One watched path, as [`Watch::add`] answered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(i32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub key: Key,
    /// The entry of a watched directory the event is about. Linux names it; kqueue never does, so
    /// `None` means "something in it", and a reader must treat it as any name.
    pub name: Option<OsString>,
    /// The watched path itself was removed or renamed away. Its key is dead from this event on.
    pub gone: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Waited {
    Changed(Vec<Event>),
    TimedOut,
    /// The [`Stop`] was dropped. Every later wait answers this at once.
    Stopped,
}

/// A set of watched paths, read by one thread at a time.
///
/// A directory reports entries created, removed and renamed in or out, and on Linux a file in it
/// closed after writing. A file reports writes to it and its own removal or rename. Nothing is
/// coalesced or debounced here: what one [`Watch::wait`] returns is what the kernel had queued.
pub struct Watch(imp::Watch);

/// Ends a [`Watch`]'s waiting when dropped, from any thread: the replacement for waking on a timer
/// to ask whether anybody still listens.
pub struct Stop(imp::Stop);

impl Watch {
    pub fn new() -> io::Result<(Self, Stop)> {
        let (watch, stop) = imp::Watch::new()?;
        Ok((Self(watch), Stop(stop)))
    }

    pub fn add(&mut self, path: &Path) -> io::Result<Key> {
        self.0.add(path)
    }

    pub fn remove(&mut self, key: Key) {
        self.0.remove(key);
    }

    /// `None` waits for as long as it takes.
    pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<Waited> {
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        loop {
            let left = deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()));
            // A watch this side took off still queues its own removal, which is news to nobody.
            match self.0.wait(left)? {
                Waited::Changed(events) if events.is_empty() => {
                    if left.is_some_and(|l| l.is_zero()) {
                        return Ok(Waited::TimedOut);
                    }
                }
                waited => return Ok(waited),
            }
        }
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Event, Key, Waited};
    use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll};
    use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
    use std::collections::HashSet;
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    pub(super) struct Watch {
        fd: OwnedFd,
        stop: Arc<OwnedFd>,
        stopped: bool,
        keys: HashSet<i32>,
    }

    pub(super) struct Stop(Arc<OwnedFd>);

    impl Stop {
        pub(super) fn stop(&self) {
            let _ = rustix::io::write(&*self.0, &1u64.to_ne_bytes());
        }
    }

    impl Watch {
        pub(super) fn new() -> io::Result<(Self, Stop)> {
            let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)?;
            let stop = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
            Ok((
                Self {
                    fd,
                    stop: stop.clone(),
                    stopped: false,
                    keys: HashSet::new(),
                },
                Stop(stop),
            ))
        }

        pub(super) fn add(&mut self, path: &Path) -> io::Result<Key> {
            let flags = if path.is_dir() {
                WatchFlags::CREATE
                    | WatchFlags::DELETE
                    | WatchFlags::MOVED_TO
                    | WatchFlags::MOVED_FROM
                    | WatchFlags::CLOSE_WRITE
            } else {
                WatchFlags::MODIFY | WatchFlags::CLOSE_WRITE
            };
            let wd = inotify::add_watch(
                &self.fd,
                path,
                flags | WatchFlags::DELETE_SELF | WatchFlags::MOVE_SELF,
            )?;
            self.keys.insert(wd);
            Ok(Key(wd))
        }

        pub(super) fn remove(&mut self, key: Key) {
            if self.keys.remove(&key.0) {
                let _ = inotify::remove_watch(&self.fd, key.0);
            }
        }

        pub(super) fn wait(&mut self, timeout: Option<Duration>) -> io::Result<Waited> {
            if self.stopped {
                return Ok(Waited::Stopped);
            }
            let spec = timeout.and_then(|t| Timespec::try_from(t).ok());
            let mut fds = [
                PollFd::new(&self.fd, PollFlags::IN),
                PollFd::new(&*self.stop, PollFlags::IN),
            ];
            loop {
                match poll(&mut fds, spec.as_ref()) {
                    Ok(0) => return Ok(Waited::TimedOut),
                    Ok(_) => break,
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            if !fds[1].revents().is_empty() {
                self.stopped = true;
                return Ok(Waited::Stopped);
            }
            Ok(Waited::Changed(self.drain()))
        }

        fn drain(&mut self) -> Vec<Event> {
            let mut buf = [MaybeUninit::<u8>::uninit(); 8192];
            let mut reader = inotify::Reader::new(&self.fd, &mut buf);
            let mut events = Vec::new();
            let mut moved_away = Vec::new();
            loop {
                let event = match reader.next() {
                    Ok(event) => event,
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(_) => break,
                };
                let wd = event.wd();
                let flags = event.events();
                if flags.contains(ReadFlags::IGNORED) {
                    if self.keys.remove(&wd) {
                        events.push(Event {
                            key: Key(wd),
                            name: None,
                            gone: true,
                        });
                    }
                    continue;
                }
                if !self.keys.contains(&wd) {
                    continue;
                }
                // A moved inode keeps its inotify watch; kqueue's is dropped with it, and one rule
                // for both is the one a caller can rely on.
                if flags.contains(ReadFlags::MOVE_SELF) {
                    moved_away.push(wd);
                }
                let gone = flags.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF);
                events.push(Event {
                    key: Key(wd),
                    name: event
                        .file_name()
                        .map(|n| std::ffi::OsStr::from_bytes(n.to_bytes()).to_os_string()),
                    gone,
                });
            }
            for wd in moved_away {
                self.remove(Key(wd));
            }
            events
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::{Event, Key, Waited};
    use rustix::event::kqueue::{
        Event as KEvent, EventFilter, EventFlags, UserDefinedFlags, UserFlags, VnodeEvents, kevent, kqueue,
    };
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    const STOP: isize = 0;

    pub(super) struct Watch {
        kq: Arc<OwnedFd>,
        stopped: bool,
        paths: HashMap<i32, OwnedFd>,
    }

    pub(super) struct Stop(Arc<OwnedFd>);

    impl Stop {
        pub(super) fn stop(&self) {
            let trigger = KEvent::new(
                EventFilter::User {
                    ident: STOP,
                    flags: UserFlags::TRIGGER,
                    user_flags: UserDefinedFlags::new(0),
                },
                EventFlags::empty(),
                std::ptr::null_mut(),
            );
            let mut none: [MaybeUninit<KEvent>; 0] = [];
            let _ = unsafe { kevent(&*self.0, &[trigger], &mut none, None) };
        }
    }

    impl Watch {
        pub(super) fn new() -> io::Result<(Self, Stop)> {
            let kq = Arc::new(kqueue()?);
            // Level-triggered once fired, so a stop is seen by every wait after it.
            let user = KEvent::new(
                EventFilter::User {
                    ident: STOP,
                    flags: UserFlags::empty(),
                    user_flags: UserDefinedFlags::new(0),
                },
                EventFlags::ADD,
                std::ptr::null_mut(),
            );
            let mut none: [MaybeUninit<KEvent>; 0] = [];
            unsafe { kevent(&*kq, &[user], &mut none, Some(Duration::ZERO))? };
            Ok((
                Self {
                    kq: kq.clone(),
                    stopped: false,
                    paths: HashMap::new(),
                },
                Stop(kq),
            ))
        }

        pub(super) fn add(&mut self, path: &Path) -> io::Result<Key> {
            let c = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
            let raw = unsafe { libc::open(c.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let watch = KEvent::new(
                EventFilter::Vnode {
                    vnode: raw,
                    flags: VnodeEvents::WRITE
                        | VnodeEvents::EXTEND
                        | VnodeEvents::DELETE
                        | VnodeEvents::RENAME
                        | VnodeEvents::REVOKE,
                },
                EventFlags::ADD | EventFlags::CLEAR,
                std::ptr::null_mut(),
            );
            let mut none: [MaybeUninit<KEvent>; 0] = [];
            // Safety: the vnode's fd is held in `paths` until its event is deleted with it.
            unsafe { kevent(&*self.kq, &[watch], &mut none, Some(Duration::ZERO))? };
            self.paths.insert(raw, fd);
            Ok(Key(raw))
        }

        pub(super) fn remove(&mut self, key: Key) {
            // Closing the fd removes its kevents.
            self.paths.remove(&key.0);
        }

        pub(super) fn wait(&mut self, timeout: Option<Duration>) -> io::Result<Waited> {
            if self.stopped {
                return Ok(Waited::Stopped);
            }
            let mut buf = [MaybeUninit::<KEvent>::uninit(); 64];
            let (got, _) = loop {
                match unsafe { kevent(&*self.kq, &[], &mut buf, timeout) } {
                    Ok(got) => break got,
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                }
            };
            if got.is_empty() {
                return Ok(Waited::TimedOut);
            }
            let mut events = Vec::new();
            for event in got.iter() {
                match event.filter() {
                    EventFilter::User { .. } => {
                        self.stopped = true;
                        return Ok(Waited::Stopped);
                    }
                    EventFilter::Vnode { vnode, flags } => {
                        if !self.paths.contains_key(&vnode) {
                            continue;
                        }
                        let gone =
                            flags.intersects(VnodeEvents::DELETE | VnodeEvents::RENAME | VnodeEvents::REVOKE);
                        events.push(Event {
                            key: Key(vnode),
                            name: None,
                            gone,
                        });
                    }
                    _ => {}
                }
            }
            for event in events.iter().filter(|e| e.gone) {
                self.paths.remove(&event.key.0);
            }
            Ok(Waited::Changed(events))
        }
    }
}
