//! The completed lines a command has written, and the unterminated one it is sitting on.
//!
//! The unterminated tail is the whole reason a supervisor reads a prompt better than a screen
//! scraper can: a prompt is text the command wrote and did not end with a newline, and that is
//! true of every CLI without any of them cooperating. A screen has no such distinction — by the
//! time bytes are cells, the newline is gone.

/// Completed lines kept for context. A fleet run's interesting output is at its end; the whole
/// transcript is the pane's own grid, which the operator can open.
const KEEP: usize = 200;

#[derive(Debug, Default)]
pub struct Tail {
    completed: Vec<String>,
    current: String,
    received: usize,
}

impl Tail {
    pub fn push(&mut self, bytes: &[u8]) {
        self.received += bytes.len();
        for chunk in String::from_utf8_lossy(bytes).chars() {
            match chunk {
                '\n' => {
                    let line = std::mem::take(&mut self.current);
                    self.completed.push(line);
                    if self.completed.len() > KEEP {
                        self.completed.drain(..self.completed.len() - KEEP);
                    }
                }
                // A progress bar rewrites its own line rather than adding one. Treating the
                // carriage return as ordinary text would leave a "prompt" made of every frame of
                // the bar concatenated.
                '\r' => self.current.clear(),
                '\u{8}' => {
                    self.current.pop();
                }
                c => self.current.push(c),
            }
        }
    }

    /// The text since the last newline — what a prompt is.
    pub fn unterminated(&self) -> &str {
        &self.current
    }

    pub fn completed(&self) -> &[String] {
        &self.completed
    }

    /// Every byte ever pushed, counted rather than measured off what was kept.
    ///
    /// The supervisor uses this to tell "the output has settled" from "more is still arriving",
    /// and the retained text cannot answer that: a progress bar rewrites one line forever without
    /// changing its length, and `completed` is capped.
    pub fn received(&self) -> usize {
        self.received
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(bytes: &str) -> Tail {
        let mut t = Tail::default();
        t.push(bytes.as_bytes());
        t
    }

    #[test]
    fn the_prompt_is_whatever_was_not_terminated() {
        let t = tail("resolving dependencies...\nlooking for conflicting packages...\n:: Proceed? [Y/n] ");
        assert_eq!(t.unterminated(), ":: Proceed? [Y/n] ");
        assert_eq!(t.completed().len(), 2);
        assert_eq!(t.completed()[0], "resolving dependencies...");
    }

    #[test]
    fn a_command_that_has_terminated_every_line_is_not_prompting() {
        let t = tail("all done\n");
        assert_eq!(t.unterminated(), "");
    }

    #[test]
    fn a_progress_bar_leaves_only_its_last_frame() {
        // pacman redraws downloads with \r. Without this the "prompt" would be every frame of the
        // bar glued end to end, and no shape would ever match it.
        let t = tail("linux-firmware  10%\rlinux-firmware  60%\rlinux-firmware 100%");
        assert_eq!(t.unterminated(), "linux-firmware 100%");
        assert!(t.completed().is_empty());
    }

    #[test]
    fn a_backspace_erases_one_character() {
        let t = tail("yess\u{8}");
        assert_eq!(t.unterminated(), "yes");
    }

    #[test]
    fn bytes_arriving_split_across_reads_join_up() {
        let mut t = Tail::default();
        t.push(b":: Proceed with ");
        t.push(b"installation? [Y/n] ");
        assert_eq!(t.unterminated(), ":: Proceed with installation? [Y/n] ");
    }

    #[test]
    fn every_byte_is_counted_even_when_the_text_it_wrote_is_gone() {
        // A progress bar rewrites one line of the same width forever. Anything derived from the
        // retained text reads as unchanged, which is the difference between "settled" and "still
        // arriving" going wrong.
        let mut t = Tail::default();
        t.push(b"aaa 10%\r");
        let first = t.received();
        t.push(b"aaa 60%\r");
        assert!(t.received() > first);
        assert_eq!(t.unterminated(), "");
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest() {
        let mut t = Tail::default();
        for i in 0..(KEEP + 50) {
            t.push(format!("line {i}\n").as_bytes());
        }
        assert_eq!(t.completed().len(), KEEP);
        assert_eq!(t.completed().last().unwrap(), &format!("line {}", KEEP + 49));
    }
}
