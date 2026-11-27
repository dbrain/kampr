use std::io;

use kampr_keep::SessionDir;

/// The process behind the conversation: a keeper's pipes in production, a recorder in a test.
pub(super) trait Child: Send {
    fn write(&mut self, line: &str) -> io::Result<()>;
    fn close(&mut self);
    fn exited(&self) -> bool;
}

pub(super) struct Keeper(pub SessionDir);

impl Child for Keeper {
    fn write(&mut self, line: &str) -> io::Result<()> {
        kampr_keep::write(&self.0, line)
    }

    fn close(&mut self) {
        let _ = kampr_keep::close(&self.0);
    }

    fn exited(&self) -> bool {
        self.0.exit().is_some()
    }
}
