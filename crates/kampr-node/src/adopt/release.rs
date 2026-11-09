use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

const FETCH: Duration = Duration::from_secs(180);

/// This node's own version as published, one verified copy per target under
/// `<state>/docker/bin/<version>/<target>/kampr`, fetched by the embedded installer.
pub struct Releases {
    dir: PathBuf,
    version: String,
    repo: String,
    base: Option<String>,
    fetching: tokio::sync::Mutex<()>,
}

impl Releases {
    pub fn new(state_dir: &Path, version: &str, repo: &str, base: Option<String>) -> Self {
        Self {
            dir: state_dir.join("docker").join("bin").join(version),
            version: version.to_string(),
            repo: repo.to_string(),
            base,
            fetching: tokio::sync::Mutex::new(()),
        }
    }

    pub fn cached(&self, target: &str) -> PathBuf {
        self.dir.join(target).join("kampr")
    }

    /// One fetch at a time, so two containers adopted together download once.
    pub async fn fetch(&self, target: &str) -> Result<PathBuf> {
        let _one = self.fetching.lock().await;
        let binary = self.cached(target);
        if binary.is_file() {
            return Ok(binary);
        }
        std::fs::create_dir_all(&self.dir).with_context(|| format!("creating {}", self.dir.display()))?;
        let script = self.dir.join("install.sh");
        std::fs::write(&script, crate::installer::SCRIPT)?;
        let mut command = crate::installer::command(&script, &self.repo);
        command
            .env("KAMPR_MODE", "fetch")
            .env("KAMPR_TARGET", target)
            .env("KAMPR_VERSION", format!("v{}", self.version))
            .env("KAMPR_PREFIX", binary.parent().expect("a target directory"));
        if let Some(base) = &self.base {
            command.env("KAMPR_BASE_URL", base);
        }
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        let out = tokio::time::timeout(FETCH, command.output())
            .await
            .with_context(|| format!("fetching kampr {} for {target} took over {FETCH:?}", self.version))?
            .context("running the installer — is `sh` on this host?")?;
        let said = String::from_utf8_lossy(&out.stderr);
        let why = said
            .lines()
            .find(|l| l.starts_with("kampr: "))
            .unwrap_or("")
            .trim_start_matches("kampr: ");
        if !out.status.success() || !binary.is_file() {
            bail!(
                "kampr {} has no {target} release that could be fetched and verified ({})",
                self.version,
                if why.is_empty() {
                    "the installer said nothing"
                } else {
                    why
                }
            );
        }
        Ok(binary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_testkit::release::Release;

    const TARGET: &str = "aarch64-unknown-linux-musl";

    fn fixture(root: &Path) -> (Release, Vec<u8>) {
        let built = root.join("built");
        let bytes = kampr_testkit::elf::static_elf(kampr_testkit::elf::ARM64);
        std::fs::write(&built, &bytes).unwrap();
        let dir = root.join("rel");
        std::fs::create_dir_all(&dir).unwrap();
        (Release::of(&dir, TARGET, &built), bytes)
    }

    #[tokio::test]
    async fn a_release_is_fetched_verified_and_kept_for_the_next_container() {
        let root = tempfile::tempdir().unwrap();
        let (release, bytes) = fixture(root.path());
        let state = root.path().join("state");
        let releases = Releases::new(&state, "0.1.139", "dbrain/kampr", Some(release.base()));

        let fetched = releases.fetch(TARGET).await.unwrap();
        assert_eq!(
            fetched,
            state.join("docker/bin/0.1.139").join(TARGET).join("kampr")
        );
        assert_eq!(std::fs::read(&fetched).unwrap(), bytes);

        std::fs::remove_dir_all(&release.dir).unwrap();
        assert_eq!(
            releases.fetch(TARGET).await.unwrap(),
            fetched,
            "kept, not fetched again"
        );
    }

    #[tokio::test]
    async fn a_download_that_does_not_match_its_checksum_is_not_kept() {
        let root = tempfile::tempdir().unwrap();
        let (release, _) = fixture(root.path());
        release.sums(&format!("{}  kampr-{TARGET}.tar.gz\n", "0".repeat(64)));
        let state = root.path().join("state");
        let releases = Releases::new(&state, "0.1.139", "dbrain/kampr", Some(release.base()));

        let refused = format!("{:#}", releases.fetch(TARGET).await.unwrap_err());
        assert!(refused.contains("checksum mismatch"), "{refused}");
        assert!(!releases.cached(TARGET).exists());
    }

    #[tokio::test]
    async fn a_version_with_nothing_published_says_which_version_and_target() {
        let root = tempfile::tempdir().unwrap();
        let (release, _) = fixture(root.path());
        let state = root.path().join("state");
        let releases = Releases::new(&state, "0.1.140", "dbrain/kampr", Some(release.base()));

        let refused = format!(
            "{:#}",
            releases.fetch("x86_64-unknown-linux-musl").await.unwrap_err()
        );
        assert!(
            refused.contains("kampr 0.1.140 has no x86_64-unknown-linux-musl release"),
            "{refused}"
        );
        assert!(refused.contains("could not download"), "{refused}");
    }
}
