use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::SystemTime;

/// The `kampr-keep` binary beside this test binary, built first when it is missing or older than
/// its sources. A crate's integration tests only get `CARGO_BIN_EXE_*` for its own binaries, and a
/// node test needs another crate's keeper to start a session.
pub fn keeper() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| built("kampr-keep", "kampr-keep", &["kampr-keep"]))
        .clone()
}

/// The `kampr` binary, for what a herdr pane runs to show a native session on the desk.
pub fn kampr() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| built("kampr-cli", "kampr", &["kampr-cli", "kampr-native", "kampr-keep"]))
        .clone()
}

fn built(package: &str, bin: &str, crates: &[&str]) -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary");
    let dir = exe.parent().expect("a target directory");
    let dir = if dir.ends_with("deps") {
        dir.parent().unwrap_or(dir)
    } else {
        dir
    };
    let binary = dir.join(bin);
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let newest_source = crates
        .iter()
        .filter_map(|c| newest(&here.join("..").join(c).join("src")))
        .max();
    if built_at(&binary).is_none_or(|at| newest_source.is_some_and(|src| src > at)) {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| env!("CARGO").into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", package, "--bin", bin])
            .status()
            .unwrap_or_else(|e| panic!("cargo build {bin}: {e}"));
        assert!(status.success(), "building {bin} failed");
    }
    assert!(binary.is_file(), "no {bin} at {}", binary.display());
    binary
}

fn built_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn newest(dir: &Path) -> Option<SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            match path.is_dir() {
                true => newest(&path),
                false => built_at(&path),
            }
        })
        .max()
}
