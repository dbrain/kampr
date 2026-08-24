//! Which `herdr` the node will actually run.
//!
//! The socket half of the node is pinned by `HERDR_SOCKET_PATH`; the binary half was a bare
//! `PATH` lookup, and a `systemd --user` manager's `PATH` is `/usr/local/bin:/usr/bin:/bin` with
//! no `~/.local/bin` in it — which is where `install.sh` puts both binaries. So a node under its
//! own service unit could serve a correct herd over the socket and never once start the observe
//! stream that carries the grid.

use std::ffi::OsString;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A path the operator (or `kampr service install`) wrote into `config.toml`.
    Configured,
    /// `HERDR_BIN_PATH`, which herdr injects into every process it spawns and which names the
    /// running server's own binary — the exactly-right answer whenever it is present.
    Injected,
    Path,
    /// The directory holding this process's own executable. `install.sh` puts kampr and herdr in
    /// the same prefix, and it is the one place a service manager's `PATH` cannot take away.
    BesideKampr,
    Prefix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFound {
    pub binary: String,
    pub tried: Vec<PathBuf>,
    pub explicit: bool,
}

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.explicit {
            return write!(
                f,
                "the herdr binary is configured as {} and that is not an executable file",
                self.binary
            );
        }
        let tried: Vec<String> = self.tried.iter().map(|p| p.display().to_string()).collect();
        write!(
            f,
            "no herdr binary: nothing named `{}` is executable on this process's PATH, beside the \
             kampr binary, or in the usual install prefixes (tried {})",
            self.binary,
            match tried.is_empty() {
                true => "nowhere — PATH is empty".to_string(),
                false => tried.join(", "),
            }
        )
    }
}

impl std::error::Error for NotFound {}

/// Every place a bare binary name is looked for, as values rather than as environment reads, so
/// the order is a table test rather than a process the suite has to fork.
#[derive(Debug, Clone, Default)]
pub struct Search {
    pub injected: Option<PathBuf>,
    pub path: Option<OsString>,
    pub beside: Option<PathBuf>,
    pub prefixes: Vec<PathBuf>,
}

impl Search {
    pub fn from_env() -> Self {
        Self {
            injected: std::env::var_os("HERDR_BIN_PATH")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            path: std::env::var_os("PATH"),
            beside: std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf)),
            prefixes: prefixes(std::env::var_os("HOME").map(PathBuf::from)),
        }
    }
}

fn prefixes(home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = home.into_iter().map(|h| h.join(".local/bin")).collect();
    dirs.push(PathBuf::from("/usr/local/bin"));
    // Not on launchd's default PATH, and where a `brew install` lands on Apple silicon.
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs
}

/// A name with a separator in it is the operator's own answer and is never searched for: the
/// suites that point `[herdr] binary` at a path that does not exist are asserting that a node
/// with no herdr binary stays a node with no herdr binary.
pub fn candidates(binary: &str, search: &Search) -> Vec<(Origin, PathBuf)> {
    if binary.contains(MAIN_SEPARATOR) {
        return vec![(Origin::Configured, PathBuf::from(binary))];
    }
    let mut found: Vec<(Origin, PathBuf)> = Vec::new();
    let mut add = |origin: Origin, path: PathBuf| {
        if !found.iter().any(|(_, seen)| seen == &path) {
            found.push((origin, path));
        }
    };
    if let Some(injected) = &search.injected {
        add(Origin::Injected, injected.clone());
    }
    if let Some(path) = &search.path {
        for dir in std::env::split_paths(path).filter(|d| !d.as_os_str().is_empty()) {
            add(Origin::Path, dir.join(binary));
        }
    }
    if let Some(beside) = &search.beside {
        add(Origin::BesideKampr, beside.join(binary));
    }
    for prefix in &search.prefixes {
        add(Origin::Prefix, prefix.join(binary));
    }
    found
}

pub fn locate(binary: &str, search: &Search) -> Result<Found, NotFound> {
    let tried = candidates(binary, search);
    for (origin, path) in &tried {
        if runnable(path) {
            return Ok(Found {
                path: path.clone(),
                origin: *origin,
            });
        }
    }
    Err(NotFound {
        binary: binary.to_string(),
        explicit: binary.contains(MAIN_SEPARATOR),
        tried: tried.into_iter().map(|(_, path)| path).collect(),
    })
}

/// What a caller that cannot report anything runs. Falling back to the configured name means the
/// spawn fails exactly as it did before rather than differently.
pub fn program(binary: &str) -> PathBuf {
    locate(binary, &Search::from_env()).map_or_else(|_| PathBuf::from(binary), |found| found.path)
}

fn runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(path: &[&str], beside: Option<&str>, prefixes: &[&str]) -> Search {
        Search {
            injected: None,
            path: Some(std::env::join_paths(path.iter().map(Path::new)).expect("a PATH")),
            beside: beside.map(PathBuf::from),
            prefixes: prefixes.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn the_search_order_is_the_injected_binary_then_path_then_the_prefix_kampr_lives_in() {
        let mut env = search(&["/a", "/b"], Some("/opt/kampr"), &["/home/x/.local/bin"]);
        env.injected = Some(PathBuf::from("/run/herdr/herdr"));
        assert_eq!(
            candidates("herdr", &env),
            vec![
                (Origin::Injected, PathBuf::from("/run/herdr/herdr")),
                (Origin::Path, PathBuf::from("/a/herdr")),
                (Origin::Path, PathBuf::from("/b/herdr")),
                (Origin::BesideKampr, PathBuf::from("/opt/kampr/herdr")),
                (Origin::Prefix, PathBuf::from("/home/x/.local/bin/herdr")),
            ]
        );
    }

    #[test]
    fn a_configured_path_is_the_whole_search() {
        for name in ["/opt/herdr", "./herdr", "bin/herdr"] {
            assert_eq!(
                candidates(name, &search(&["/a"], Some("/b"), &["/c"])),
                vec![(Origin::Configured, PathBuf::from(name))],
                "a name with a separator in it is not a name to search for"
            );
        }
    }

    #[test]
    fn a_directory_reachable_two_ways_is_tried_once() {
        let env = search(
            &["/usr/local/bin", "/home/x/.local/bin"],
            Some("/home/x/.local/bin"),
            &["/home/x/.local/bin", "/usr/local/bin"],
        );
        let tried: Vec<PathBuf> = candidates("herdr", &env).into_iter().map(|(_, p)| p).collect();
        assert_eq!(
            tried,
            vec![
                PathBuf::from("/usr/local/bin/herdr"),
                PathBuf::from("/home/x/.local/bin/herdr"),
            ]
        );
    }

    #[test]
    fn an_empty_environment_has_nowhere_to_look_and_says_so() {
        let nothing = Search::default();
        assert!(candidates("herdr", &nothing).is_empty());
        let error = locate("herdr", &nothing).expect_err("nothing to find");
        assert!(error.to_string().contains("PATH is empty"), "{error}");
    }

    #[test]
    fn the_home_prefix_comes_before_the_system_ones() {
        assert_eq!(
            prefixes(Some(PathBuf::from("/home/x"))),
            vec![
                PathBuf::from("/home/x/.local/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ]
        );
        assert_eq!(prefixes(None).len(), 2, "no HOME is not a panic");
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).expect("a directory");
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("a file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        path
    }

    /// The operator's own machine: a service whose PATH is `/usr/local/bin:/usr/bin:/bin` and a
    /// herdr in `~/.local/bin`, next to the kampr the unit runs.
    #[test]
    fn a_binary_beside_kampr_is_found_with_nothing_on_the_path() {
        let home = tempfile::tempdir().expect("a home");
        let bin = home.path().join(".local/bin");
        let herdr = executable(&bin, "herdr");
        let env = Search {
            injected: None,
            path: Some(std::env::join_paths(["/usr/bin", "/bin"]).expect("a PATH")),
            beside: Some(bin.clone()),
            prefixes: Vec::new(),
        };
        assert_eq!(
            locate("herdr", &env).expect("a herdr next to kampr"),
            Found {
                path: herdr,
                origin: Origin::BesideKampr
            }
        );
    }

    #[test]
    fn a_file_that_is_not_executable_is_not_a_binary() {
        let dir = tempfile::tempdir().expect("a dir");
        let herdr = dir.path().join("herdr");
        std::fs::write(&herdr, "not a program").expect("a file");
        let env = Search {
            beside: Some(dir.path().to_path_buf()),
            ..Search::default()
        };
        let error = locate("herdr", &env).expect_err("a text file is not herdr");
        assert_eq!(error.tried, vec![herdr]);
        assert!(!error.explicit);
    }

    #[test]
    fn a_configured_path_that_is_gone_names_itself_rather_than_the_search() {
        let dir = tempfile::tempdir().expect("a dir");
        let missing = dir.path().join("herdr");
        let error =
            locate(&missing.display().to_string(), &Search::from_env()).expect_err("nothing at that path");
        assert!(error.explicit);
        let said = error.to_string();
        assert!(said.contains(&missing.display().to_string()), "{said}");
        assert!(!said.contains("PATH"), "{said}");
    }
}
