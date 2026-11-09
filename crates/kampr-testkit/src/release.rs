//! A release on disk in the shape the release workflow publishes, for `install.sh` to fetch from
//! a `file://` base: `kampr-<target>.tar.gz` holding `kampr`, and a `SHA256SUMS` naming it.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Release {
    pub dir: PathBuf,
}

impl Release {
    pub fn of(dir: &Path, target: &str, binary: &Path) -> Self {
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).expect("a stage dir");
        std::fs::copy(binary, stage.join("kampr")).expect("the binary staged");
        let asset = format!("kampr-{target}.tar.gz");
        let tar = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&asset))
            .arg("-C")
            .arg(&stage)
            .arg("kampr")
            .status()
            .expect("tar");
        assert!(tar.success(), "could not build {asset}");
        std::fs::remove_dir_all(&stage).expect("the stage cleared");
        let release = Self {
            dir: dir.to_path_buf(),
        };
        release.sums(&format!("{}  {asset}\n", sha256(&dir.join(&asset))));
        release
    }

    pub fn base(&self) -> String {
        format!("file://{}", self.dir.display())
    }

    pub fn sums(&self, text: &str) {
        std::fs::write(self.dir.join("SHA256SUMS"), text).expect("SHA256SUMS");
    }
}

fn sha256(path: &Path) -> String {
    let out = Command::new("sha256sum").arg(path).output().expect("sha256sum");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}
