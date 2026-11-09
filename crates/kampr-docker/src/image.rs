use crate::Client;
use anyhow::{Result, bail};

pub async fn ensure(client: &Client, reference: &str) -> Result<()> {
    if client.has_image(reference).await? {
        return Ok(());
    }
    let (from_image, tag) = split(reference);
    let progress = client.create_image(from_image, tag).await?;
    match failure(&progress) {
        Some(message) => bail!("{message}"),
        None => Ok(()),
    }
}

/// An untagged pull with no `tag` fetches every tag of the repository, so one is always named.
fn split(reference: &str) -> (&str, &str) {
    if reference.contains('@') {
        return (reference, "");
    }
    match reference.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => (name, tag),
        _ => (reference, "latest"),
    }
}

fn failure(progress: &[u8]) -> Option<String> {
    progress
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .find_map(|line| line["error"].as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pull_always_names_one_tag() {
        for (reference, want) in [
            ("busybox:1.37-musl", ("busybox", "1.37-musl")),
            ("busybox", ("busybox", "latest")),
            ("registry:5000/team/tool", ("registry:5000/team/tool", "latest")),
            ("registry:5000/team/tool:2", ("registry:5000/team/tool", "2")),
            ("busybox@sha256:abc", ("busybox@sha256:abc", "")),
        ] {
            assert_eq!(split(reference), want, "{reference}");
        }
    }

    #[test]
    fn a_pull_that_failed_part_way_says_why() {
        let ok = b"{\"status\":\"Pulling from library/busybox\"}\n{\"status\":\"Downloaded newer image\"}\n";
        assert_eq!(failure(ok), None);
        let failed = b"{\"status\":\"Pulling fs layer\"}\n{\"errorDetail\":{\"message\":\"no space\"},\"error\":\"no space\"}\n";
        assert_eq!(failure(failed).as_deref(), Some("no space"));
    }
}
