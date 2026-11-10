use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::SystemTime;

/// The `kampr-keep` binary in the target dir this test was built for, built first when it is missing or older than
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
    let binary = target_binary(bin);
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

/// Where cargo puts `bin` for the profile this test was built with, whether or not it is built yet.
pub fn target_binary(bin: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary");
    artifact_dir(&exe, &target_dir()).join(bin)
}

pub(crate) fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
}

/// A configured `build.build-dir` keeps test binaries apart from the target dir that final binaries
/// are uplifted to, so the test binary's path says only which profile (and triple) to look in.
fn artifact_dir(exe: &Path, target: &Path) -> PathBuf {
    let dir = exe.parent().expect("a build directory");
    let profile = if dir.ends_with("deps") {
        dir.parent().unwrap_or(dir)
    } else {
        dir
    };
    match profile
        .ancestors()
        .find(|d| d.join("CACHEDIR.TAG").is_file())
        .and_then(|root| profile.strip_prefix(root).ok())
    {
        Some(rel) => target.join(rel),
        None => profile.to_path_buf(),
    }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn looks_in_the_target_dir_for_the_test_binarys_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        let build = tmp.path().join("build");
        for root in [&target, &build] {
            std::fs::create_dir_all(root).unwrap();
            std::fs::write(root.join("CACHEDIR.TAG"), "").unwrap();
        }
        for (exe, want) in [
            (target.join("debug/deps/it-0123"), "debug"),
            (build.join("debug/deps/it-0123"), "debug"),
            (build.join("release/deps/it-0123"), "release"),
            (
                build.join("x86_64-unknown-linux-musl/debug/deps/it-0123"),
                "x86_64-unknown-linux-musl/debug",
            ),
        ] {
            assert_eq!(
                super::artifact_dir(&exe, &target),
                target.join(want),
                "{}",
                exe.display()
            );
        }
        let untagged = Path::new("/nowhere/debug/deps/it-0123");
        assert_eq!(
            super::artifact_dir(untagged, &target),
            Path::new("/nowhere/debug")
        );
    }

    #[test]
    fn finds_the_binaries_cargo_built() {
        for path in [super::keeper(), super::kampr()] {
            assert!(path.is_file(), "{}", path.display());
        }
    }
}
