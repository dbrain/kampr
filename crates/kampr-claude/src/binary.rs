use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use kampr_core::conversation::Refusal;

/// The `[claude] binary` as a path the keeper can exec. A keeper starts its child with the
/// node's own `PATH`, which as a service lacks `~/.local/bin` where Claude installs itself, so a
/// bare name is looked up on the login environment's `PATH` first, as a person's terminal would.
pub fn resolve(name: &str, login: &[(String, String)]) -> Result<String, Refusal> {
    if name.contains('/') {
        return runnable(Path::new(name))
            .then(|| name.to_string())
            .ok_or_else(|| {
                Refusal::new(format!(
                    "the configured Claude binary {name} is not an executable file"
                ))
            });
    }
    let node = std::env::var("PATH").ok();
    if let Some(found) = [kampr_core::login::path_of(login), node.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|path| on(name, path))
    {
        return Ok(found);
    }
    Err(Refusal::new(format!(
        "`{name}` is on neither this node's PATH nor its login shell's; set [claude] binary to its full path"
    )))
}

fn on(name: &str, path: &str) -> Option<String> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|p| runnable(p))
        .map(|p: PathBuf| p.display().to_string())
}

pub(crate) fn runnable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_bare_name_is_found_on_the_login_path_and_a_missing_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let found = dir.path().join("kampr-test-claude");
        executable(&found, "");
        let login = [("PATH".to_string(), format!("/nowhere:{}", dir.path().display()))];
        assert_eq!(
            resolve("kampr-test-claude", &login).ok(),
            Some(found.display().to_string())
        );

        let refused = resolve("kampr-no-such-claude", &login).unwrap_err();
        assert!(
            refused.message.contains("kampr-no-such-claude"),
            "{}",
            refused.message
        );

        let plain = dir.path().join("plain");
        std::fs::write(&plain, "").unwrap();
        assert!(resolve(&plain.display().to_string(), &login).is_err());
        assert!(resolve(&found.display().to_string(), &[]).is_ok());
    }
}
