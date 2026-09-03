//! What the operator asked for, and what gets `exec`ed for it.
//!
//! **There is a shell between the operator and the command, and it is not a login or an
//! interactive one.** That sentence is three separate decisions and each is a measurement.
//!
//! *A shell*, because the operator asked to type what they would type in bash — `&&`, `|`, `;`,
//! globs, quotes, `~`, redirection — and every one of those is the shell's, not `execve`'s.
//!
//! *Not interactive*, because [`crate::waiting::mode_of`] is the rung that survives the privilege
//! wall (#339), and #333 measured that ble.sh leaves an **interactive** shell's tty with ECHO
//! already off before anything asks for a secret. Measured on this machine, with ble.sh installed
//! and sourced from `.bashrc`: a `bash -c` on a fleet pty reads `BLE_VERSION` unset and `$-` as
//! `hBc` — no `i`, so `.bashrc` returns at its own guard — and its termios reads `ECHO on,
//! ICANON on` at idle and `ECHO OFF, ICANON on` at a real password prompt, which is exactly what
//! a pty with nothing on it reads (#337). `bash -i` on the same pty reads `ECHO OFF, ICANON OFF`
//! while merely sitting at its prompt, which is #333 reproduced. The confound needs the `i`.
//!
//! *Not a login shell*, because running one per command would re-read the profile on every host
//! on every run, and interleave whatever it prints with the run's own output. The profile is read
//! **once** per node process, for its `PATH` alone, and [`crate::env`] holds that.
//!
//! What this does not buy, and the docs say so rather than letting the operator find out: aliases
//! and shell functions. Both live in `.bashrc`, which only an interactive shell reads.

/// One instruction from the operator.
///
/// The two arms are the wire's two shapes, and the older one is not deprecated: a client that has
/// never heard of `command` goes on sending `args`, and its argv goes on being `exec`ed with
/// nothing in front of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// An argv, `exec`ed directly. What `fleet.run`'s `args` has always meant.
    Argv(Vec<String>),
    /// A command line, handed to `<shell> -c`. What `fleet.run`'s `command` means.
    Shell(String),
}

impl Job {
    /// What the operator typed, for the pane's label, the book and the boards.
    ///
    /// Never the wrapped argv. A board that says `/usr/bin/bash -c pacman -Syu` has put this
    /// module's implementation in front of the operator's own sentence, and the shell that ran it
    /// is not a thing they asked about.
    pub fn line(&self) -> String {
        match self {
            Job::Argv(argv) => argv.join(" "),
            Job::Shell(line) => line.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Job::Argv(argv) => argv.is_empty(),
            Job::Shell(line) => line.trim().is_empty(),
        }
    }

    /// What is actually `exec`ed. `shell` is [`crate::env::login_shell`]'s answer — the operator's
    /// own, so their bashisms work — invoked with neither `-l` nor `-i`.
    pub fn argv(&self, shell: &str) -> Vec<String> {
        match self {
            Job::Argv(argv) => argv.clone(),
            Job::Shell(line) => vec![shell.to_string(), "-c".to_string(), line.clone()],
        }
    }

    /// What [`crate::secretish`] reads. A shell line is one argument and that rule already
    /// flattens on whitespace, which is how it has always read `sh -c 'TOKEN=abc ./deploy'`.
    pub fn words(&self) -> Vec<String> {
        match self {
            Job::Argv(argv) => argv.clone(),
            Job::Shell(line) => vec![line.clone()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_job_is_run_by_the_operators_own_shell_with_neither_l_nor_i() {
        let argv = Job::Shell("ls | wc -l".into()).argv("/usr/bin/bash");
        assert_eq!(argv, vec!["/usr/bin/bash", "-c", "ls | wc -l"]);
        assert!(
            !argv[1].contains('i') && !argv[1].contains('l'),
            "an interactive shell reads .bashrc and brings #333's ECHO confound onto the pty",
        );
    }

    #[test]
    fn an_argv_job_is_execed_with_nothing_in_front_of_it() {
        let job = Job::Argv(vec!["pacman".into(), "-Syu".into()]);
        assert_eq!(job.argv("/usr/bin/bash"), vec!["pacman", "-Syu"]);
    }

    /// The label, the book and every board show the operator's own sentence and never the
    /// wrapper.
    #[test]
    fn what_is_shown_is_what_was_typed_and_not_the_shell_that_runs_it() {
        assert_eq!(Job::Shell("make -j8 && ./run".into()).line(), "make -j8 && ./run");
        assert_eq!(Job::Argv(vec!["uptime".into()]).line(), "uptime");
    }

    #[test]
    fn a_blank_line_is_as_empty_as_an_empty_argv() {
        assert!(Job::Shell("   ".into()).is_empty());
        assert!(Job::Argv(Vec::new()).is_empty());
        assert!(!Job::Shell("uptime".into()).is_empty());
    }
}
