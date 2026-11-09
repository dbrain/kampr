use super::binary::Binaries;
use anyhow::Result;
use kampr_docker::api::Inspect;
use kampr_docker::{ApiError, Client, archive};
use std::path::Path;

pub const BINARY: &str = "/.kampr/kampr";
const VERSION: &str = "/.kampr/version";

/// Copies the kampr that runs there into a running container unless it holds that one already.
/// True when it copied; a refusal is a sentence for a person.
pub async fn prepare(client: &Client, binaries: &Binaries, container: &Inspect) -> Result<bool, String> {
    let name = container.short_name();
    writable(container)?;
    let arch = client
        .image_architecture(&container.image)
        .await
        .map_err(|e| format!("could not read the architecture of {name}'s image: {e:#}"))?;
    let binary = binaries.for_arch(&arch, name).await?;
    ensure(client, container, &binary)
        .await
        .map_err(|e| copy_failed(name, &e))
}

pub fn writable(container: &Inspect) -> Result<(), String> {
    match container.host_config.readonly_rootfs {
        true => Err(format!(
            "{} has a read-only root filesystem (read_only: true), so kampr can't be copied into \
             it — adopting needs a writable /. Shell in still works.",
            container.short_name()
        )),
        false => Ok(()),
    }
}

async fn ensure(client: &Client, container: &Inspect, binary: &Path) -> Result<bool> {
    let version = archive::version_of(binary)?;
    let held = match client.get_archive(&container.id, VERSION).await? {
        Some(tar) => archive::only_file(&tar).ok(),
        None => None,
    };
    if held.as_deref() == Some(version.as_bytes()) {
        return Ok(false);
    }
    let bytes = tokio::fs::read(binary).await?;
    let tar = tokio::task::spawn_blocking(move || archive::injection(&bytes, &version)).await??;
    client.put_archive(&container.id, "/", tar).await?;
    Ok(true)
}

fn copy_failed(name: &str, e: &anyhow::Error) -> String {
    let said = match e.downcast_ref::<ApiError>() {
        Some(api) => api.message.clone(),
        None => format!("{e:#}"),
    };
    let lower = said.to_lowercase();
    let cause = if lower.contains("read-only") {
        "its root filesystem is read-only, so adopting needs a writable /".to_string()
    } else if lower.contains("no space left") {
        "it has no space left for kampr (about 20 MB)".to_string()
    } else if lower.contains("permission denied") {
        "it refused the write to /.kampr (permission denied)".to_string()
    } else if lower.contains("is not running") || lower.contains("no such container") {
        "it stopped or went away while kampr was being copied".to_string()
    } else {
        said
    };
    format!("kampr could not be copied into {name}: {cause}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_copy_names_the_container_and_the_cause_in_words() {
        let api = |message: &str| {
            anyhow::Error::new(ApiError {
                status: 500,
                message: message.into(),
            })
        };
        for (said, want) in [
            (
                "container rootfs is marked read-only",
                "kampr could not be copied into db: its root filesystem is read-only",
            ),
            (
                "write /.kampr/kampr: no space left on device",
                "kampr could not be copied into db: it has no space left",
            ),
            (
                "open /.kampr/kampr: permission denied",
                "kampr could not be copied into db: it refused the write to /.kampr (permission denied)",
            ),
            (
                "Container 3f2a is not running",
                "kampr could not be copied into db: it stopped or went away",
            ),
            (
                "something new",
                "kampr could not be copied into db: something new",
            ),
        ] {
            let told = copy_failed("db", &api(said));
            assert!(told.starts_with(want), "{said}: {told}");
        }
        let io = anyhow::Error::new(std::io::Error::other("broken pipe"));
        assert_eq!(
            copy_failed("db", &io),
            "kampr could not be copied into db: broken pipe"
        );
    }

    #[test]
    fn a_read_only_root_is_refused_before_anything_is_copied() {
        let mut container: Inspect = serde_json::from_str(r#"{"Id":"f00","Name":"/db"}"#).unwrap();
        assert_eq!(writable(&container), Ok(()));
        container.host_config.readonly_rootfs = true;
        let refused = writable(&container).unwrap_err();
        assert!(
            refused.starts_with("db has a read-only root filesystem (read_only: true)"),
            "{refused}"
        );
        assert!(refused.ends_with("Shell in still works."), "{refused}");
    }
}
