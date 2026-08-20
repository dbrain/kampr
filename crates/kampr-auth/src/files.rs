use std::path::Path;

/// The state directory holds the device database, its write-ahead log and the audit log. Every
/// one of them is as sensitive as the terminals it guards, so the directory is the first thing
/// that gets locked down rather than an afterthought per file.
#[cfg(unix)]
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    chmod(dir, 0o700)
}

#[cfg(unix)]
pub fn touch_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    chmod(path, 0o600)
}

#[cfg(unix)]
pub fn chmod(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(not(unix))]
pub fn touch_private(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
pub fn chmod(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}
