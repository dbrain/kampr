use crate::secret;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use std::path::Path;

/// The node's own long-lived identity, distinct from any user credential.
///
/// Node-to-node auth has to be separate from user auth, or a compromised viewer session could
/// impersonate a node to the rest of the herd. This is that identity: generated once at
/// `kampr init`, kept at 0600, and shown as a fingerprint so a second node can be pinned to it.
#[derive(Debug, Clone)]
pub struct NodeIdentity {
    signing: SigningKey,
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0} is not a 32-byte node key")]
    Malformed(String),
    #[error(transparent)]
    Random(#[from] secret::RandomError),
}

impl NodeIdentity {
    pub fn load_or_create(path: &Path) -> Result<Self, IdentityError> {
        if let Some(existing) = Self::load(path)? {
            return Ok(existing);
        }
        let bytes: [u8; 32] = secret::random_bytes(32)?
            .try_into()
            .map_err(|_| IdentityError::Malformed(path.display().to_string()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| IdentityError::Io(dir.display().to_string(), e))?;
        }
        write_private(path, &hex::encode(bytes))
            .map_err(|e| IdentityError::Io(path.display().to_string(), e))?;
        Ok(Self {
            signing: SigningKey::from_bytes(&bytes),
        })
    }

    pub fn load(path: &Path) -> Result<Option<Self>, IdentityError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(IdentityError::Io(path.display().to_string(), e)),
        };
        let bytes: [u8; 32] = hex::decode(text.trim())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| IdentityError::Malformed(path.display().to_string()))?;
        Ok(Some(Self {
            signing: SigningKey::from_bytes(&bytes),
        }))
    }

    /// Short enough to read aloud, long enough to pin.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.signing.verifying_key().to_bytes());
        hex::encode(&digest[..8])
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join("-")
    }
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_is_created_once_and_then_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key");
        let first = NodeIdentity::load_or_create(&path).unwrap();
        let second = NodeIdentity::load_or_create(&path).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_ne!(
            first.fingerprint(),
            NodeIdentity::load_or_create(&dir.path().join("other.key"))
                .unwrap()
                .fingerprint()
        );
        assert_eq!(first.fingerprint().len(), 19);
    }

    #[test]
    fn the_private_key_is_not_world_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key");
        NodeIdentity::load_or_create(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_corrupt_key_file_is_an_error_rather_than_a_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key");
        std::fs::write(&path, "not-a-key").unwrap();
        assert!(matches!(
            NodeIdentity::load_or_create(&path),
            Err(IdentityError::Malformed(_))
        ));
    }
}
