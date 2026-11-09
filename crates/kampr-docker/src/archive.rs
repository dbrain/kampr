use anyhow::{Context, Result};
use std::io::Read;

pub const DIR: &str = ".kampr";
pub const BINARY: &str = ".kampr/kampr";
pub const VERSION: &str = ".kampr/version";

/// What `PUT /containers/{id}/archive?path=/` unpacks into `/.kampr`: the directory first, so a
/// container that never had one gets it from the archive rather than from a `mkdir`.
pub fn injection(binary: &[u8], version: &str) -> Result<Vec<u8>> {
    let mut tar = tar::Builder::new(Vec::with_capacity(binary.len() + 4096));
    tar.mode(tar::HeaderMode::Deterministic);
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_mode(0o755);
    dir.set_size(0);
    tar.append_data(&mut dir, format!("{DIR}/"), std::io::empty())?;
    for (path, bytes, mode) in [(BINARY, binary, 0o755), (VERSION, version.as_bytes(), 0o644)] {
        let mut file = tar::Header::new_gnu();
        file.set_entry_type(tar::EntryType::Regular);
        file.set_mode(mode);
        file.set_size(bytes.len() as u64);
        tar.append_data(&mut file, path, bytes)?;
    }
    Ok(tar.into_inner()?)
}

/// The one file `GET /containers/{id}/archive?path=<file>` wraps in a tar of its own.
pub fn only_file(archive: &[u8]) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(archive);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_file() {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            return Ok(bytes);
        }
    }
    anyhow::bail!("the archive held no file")
}

/// Size and modification time, which change with every build or upgrade of the host's binary.
pub fn version_of(path: &std::path::Path) -> Result<String> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let modified = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    Ok(format!("{} {modified}", meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn unpacked(archive: &[u8]) -> BTreeMap<String, (tar::EntryType, u32, Vec<u8>)> {
        let mut tar = tar::Archive::new(archive);
        tar.entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let header = entry.header().clone();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                let path = entry.path().unwrap().display().to_string();
                (path, (header.entry_type(), header.mode().unwrap(), bytes))
            })
            .collect()
    }

    #[test]
    fn an_injection_makes_the_directory_and_carries_an_executable_and_its_version() {
        let binary: Vec<u8> = (0..70_000u32).map(|n| (n % 251) as u8).collect();
        let archive = injection(&binary, "123 456").unwrap();
        let entries = unpacked(&archive);
        let paths: Vec<&str> = entries.keys().map(String::as_str).collect();
        assert_eq!(paths, [".kampr/", ".kampr/kampr", ".kampr/version"]);
        assert_eq!(entries[".kampr/"].0, tar::EntryType::Directory);
        assert_eq!(entries[".kampr/"].1, 0o755);
        assert_eq!(entries[".kampr/kampr"].1, 0o755);
        assert_eq!(entries[".kampr/kampr"].2, binary);
        assert_eq!(entries[".kampr/version"].2, b"123 456");
        assert_eq!(only_file(&archive).unwrap(), binary);
    }

    #[test]
    fn a_version_changes_when_the_binary_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kampr");
        std::fs::write(&path, b"one").unwrap();
        let first = version_of(&path).unwrap();
        assert_eq!(first, version_of(&path).unwrap());
        std::fs::write(&path, b"three").unwrap();
        assert_ne!(first, version_of(&path).unwrap());
        assert!(version_of(&dir.path().join("missing")).is_err());
    }
}
