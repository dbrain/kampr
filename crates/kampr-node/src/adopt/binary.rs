use super::release::Releases;
use kampr_docker::elf;
use std::path::{Path, PathBuf};

/// Which kampr a container of a given architecture is given: the configured binary, this node's
/// own, the cached release, then a download of it, the first that would run there.
pub struct Binaries {
    pub configured: Option<PathBuf>,
    pub own: Option<PathBuf>,
    pub fetch: bool,
    pub releases: Releases,
}

impl Binaries {
    /// `arch` is the image's, empty when the daemon did not say.
    pub async fn for_arch(&self, arch: &str, container: &str) -> Result<PathBuf, String> {
        let mut why = Vec::new();
        for local in [&self.configured, &self.own].into_iter().flatten() {
            match runs(local, arch) {
                Ok(()) => return Ok(local.clone()),
                Err(e) => why.push(e),
            }
        }
        let wanted = match arch {
            "" => std::env::consts::ARCH,
            arch => arch,
        };
        let refused = |why: &[String], tail: String| match why {
            [] => format!("nothing kampr can copy would run in {container}: {tail}"),
            why => format!(
                "nothing kampr can copy would run in {container}: {}; {tail}",
                why.join("; ")
            ),
        };
        let Some(target) = elf::musl_target(wanted) else {
            return Err(refused(&why, format!("no kampr release is built for {wanted}")));
        };
        let cached = self.releases.cached(target);
        if runs(&cached, arch).is_ok() {
            return Ok(cached);
        }
        if !self.fetch {
            return Err(refused(
                &why,
                format!(
                    "set [docker] binary to a static kampr for {wanted}, or remove [docker] fetch = false"
                ),
            ));
        }
        match self.releases.fetch(target).await {
            Ok(fetched) => runs(&fetched, arch)
                .map(|()| fetched)
                .map_err(|e| refused(&[e], "the fetched release does not match its target".into())),
            Err(e) => {
                why.push(format!("{e:#}"));
                Err(refused(
                    &why,
                    format!("set [docker] binary to a static kampr built for {wanted}"),
                ))
            }
        }
    }
}

fn runs(binary: &Path, arch: &str) -> Result<(), String> {
    let found = elf::read(binary).map_err(|e| format!("{}: {e}", binary.display()))?;
    if found.interpreted {
        return Err(format!("{} is dynamically linked", binary.display()));
    }
    if !arch.is_empty() && !elf::same_arch(arch, &found.arch) {
        return Err(format!(
            "{} is {} and the image is {arch}",
            binary.display(),
            found.arch
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_testkit::elf::{AMD64, ARM64, dynamic_elf, static_elf};
    use kampr_testkit::release::Release;

    struct Machine {
        root: tempfile::TempDir,
    }

    impl Machine {
        fn new() -> Self {
            Self {
                root: tempfile::tempdir().unwrap(),
            }
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn releases(&self, base: Option<String>) -> Releases {
            Releases::new(&self.root.path().join("state"), "0.1.139", "dbrain/kampr", base)
        }

        fn cache(&self, target: &str, bytes: &[u8]) -> PathBuf {
            let cached = self.releases(None).cached(target);
            std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
            std::fs::write(&cached, bytes).unwrap();
            cached
        }

        fn published(&self, target: &str, bytes: &[u8]) -> String {
            let built = self.file("published", bytes);
            let dir = self.root.path().join("rel");
            std::fs::create_dir_all(&dir).unwrap();
            Release::of(&dir, target, &built).base()
        }

        fn nowhere(&self) -> String {
            format!("file://{}", self.root.path().join("no-release").display())
        }
    }

    fn binaries(
        configured: Option<PathBuf>,
        own: Option<PathBuf>,
        fetch: bool,
        releases: Releases,
    ) -> Binaries {
        Binaries {
            configured,
            own,
            fetch,
            releases,
        }
    }

    #[tokio::test]
    async fn the_first_binary_that_would_run_in_the_container_is_the_one_copied() {
        let m = Machine::new();
        let set_amd = m.file("set-amd", &static_elf(AMD64));
        let set_glibc = m.file("set-glibc", &dynamic_elf(AMD64));
        let own_amd = m.file("own-amd", &static_elf(AMD64));
        let own_glibc = m.file("own-glibc", &dynamic_elf(AMD64));
        let cached_arm = m.cache("aarch64-unknown-linux-musl", &static_elf(ARM64));
        let cached_amd = m.cache("x86_64-unknown-linux-musl", &static_elf(AMD64));

        for (configured, own, arch, want) in [
            (Some(&set_amd), Some(&own_amd), "amd64", &set_amd),
            (Some(&set_glibc), Some(&own_amd), "amd64", &own_amd),
            (None, Some(&own_amd), "x86_64", &own_amd),
            (Some(&set_glibc), Some(&own_glibc), "amd64", &cached_amd),
            (Some(&set_amd), Some(&own_amd), "arm64", &cached_arm),
            (None, Some(&own_amd), "", &own_amd),
        ] {
            let b = binaries(
                configured.cloned(),
                own.cloned(),
                false,
                m.releases(Some(m.nowhere())),
            );
            assert_eq!(
                b.for_arch(arch, "db").await.as_ref(),
                Ok(want),
                "{configured:?} {own:?} {arch}"
            );
        }
    }

    #[tokio::test]
    async fn with_nothing_local_or_cached_the_release_is_fetched_unless_fetching_is_off() {
        let m = Machine::new();
        let glibc = m.file("glibc", &dynamic_elf(AMD64));
        let base = m.published("aarch64-unknown-linux-musl", &static_elf(ARM64));

        let off = binaries(Some(glibc.clone()), None, false, m.releases(Some(base.clone())));
        let refused = off.for_arch("arm64", "db").await.unwrap_err();
        assert!(refused.contains("would run in db"), "{refused}");
        assert!(
            refused.contains(&format!("{} is dynamically linked", glibc.display())),
            "{refused}"
        );
        assert!(refused.contains("[docker] fetch = false"), "{refused}");

        let on = binaries(Some(glibc), None, true, m.releases(Some(base)));
        let fetched = on.for_arch("arm64", "db").await.unwrap();
        assert_eq!(fetched, m.releases(None).cached("aarch64-unknown-linux-musl"));
        assert_eq!(std::fs::read(fetched).unwrap(), static_elf(ARM64));
    }

    #[tokio::test]
    async fn a_version_with_no_release_is_refused_naming_it_and_the_way_out() {
        let m = Machine::new();
        let glibc = m.file("glibc", &dynamic_elf(AMD64));
        let b = binaries(Some(glibc), None, true, m.releases(Some(m.nowhere())));
        let refused = b.for_arch("amd64", "db").await.unwrap_err();
        assert!(
            refused.contains("kampr 0.1.139 has no x86_64-unknown-linux-musl release"),
            "{refused}"
        );
        assert!(refused.contains("set [docker] binary"), "{refused}");

        let odd = binaries(None, None, true, m.releases(Some(m.nowhere())));
        let refused = odd.for_arch("riscv64", "db").await.unwrap_err();
        assert!(
            refused.contains("no kampr release is built for riscv64"),
            "{refused}"
        );
    }
}
