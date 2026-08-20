use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jwt_simple::algorithms::ES256KeyPair;
use kampr_auth::files;
use std::path::Path;
use web_push::{PartialVapidSignatureBuilder, SubscriptionInfo, VapidSignature, VapidSignatureBuilder};

#[derive(Debug, thiserror::Error)]
pub enum VapidError {
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("generating a VAPID key: {0}")]
    Generate(String),
    #[error("{0} is not a VAPID private key: {1}")]
    Malformed(String, String),
}

/// The node's VAPID identity: one P-256 keypair, generated at `kampr init` and kept beside the
/// device database at 0600.
///
/// It is a long-lived identity, not a secret per subscription: a browser stores the public half
/// inside the subscription it hands back, so **rotating this key invalidates every subscription
/// already issued**. That is why it is written once and loaded thereafter.
pub struct Vapid {
    pem: String,
    public_key: Vec<u8>,
    subject: String,
}

impl std::fmt::Debug for Vapid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vapid")
            .field("public_key", &self.public_key_b64())
            .field("subject", &self.subject)
            .finish()
    }
}

impl Vapid {
    pub fn load_or_create(path: &Path, subject: &str) -> Result<Self, VapidError> {
        if let Some(existing) = Self::load(path, subject)? {
            return Ok(existing);
        }
        let pem = ES256KeyPair::generate()
            .to_pem()
            .map_err(|e| VapidError::Generate(e.to_string()))?;
        if let Some(dir) = path.parent() {
            files::private_dir(dir).map_err(|e| VapidError::Io(dir.display().to_string(), e))?;
        }
        files::touch_private(path).map_err(|e| VapidError::Io(path.display().to_string(), e))?;
        std::fs::write(path, &pem).map_err(|e| VapidError::Io(path.display().to_string(), e))?;
        files::chmod(path, 0o600).map_err(|e| VapidError::Io(path.display().to_string(), e))?;
        Self::from_pem(pem, subject, &path.display().to_string())
    }

    pub fn load(path: &Path, subject: &str) -> Result<Option<Self>, VapidError> {
        let pem = match std::fs::read_to_string(path) {
            Ok(pem) => pem,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(VapidError::Io(path.display().to_string(), e)),
        };
        Self::from_pem(pem, subject, &path.display().to_string()).map(Some)
    }

    /// The public half is read back **through the same signer that will sign the JWT**, rather
    /// than derived alongside it. A client stores this key inside its subscription and the push
    /// service checks the signature against it, so the two cannot be allowed to drift.
    fn from_pem(pem: String, subject: &str, whence: &str) -> Result<Self, VapidError> {
        let public_key = Self::partial(&pem)
            .map_err(|e| VapidError::Malformed(whence.to_string(), e))?
            .get_public_key();
        Ok(Self {
            pem,
            public_key,
            subject: subject.to_string(),
        })
    }

    fn partial(pem: &str) -> Result<PartialVapidSignatureBuilder, String> {
        VapidSignatureBuilder::from_pem_no_sub(pem.as_bytes()).map_err(|e| e.to_string())
    }

    /// What a browser passes as `applicationServerKey`.
    pub fn public_key_b64(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.public_key)
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The `sub` claim is not decoration: a push service rejects a VAPID JWT without one, so it
    /// is set here rather than left to a default that does not exist.
    pub fn sign(&self, info: &SubscriptionInfo) -> Result<VapidSignature, web_push::WebPushError> {
        let mut builder = VapidSignatureBuilder::from_pem(self.pem.as_bytes(), info)?;
        builder.add_claim("sub", self.subject.as_str());
        builder.build()
    }
}

/// A VAPID `sub` must be a `mailto:` or `https:` URI identifying whoever runs the server. A node
/// on a hostname can name itself; one on a LAN IP has no such name, and `mailto:` is the honest
/// fallback rather than an `https://192.168…` that no push service would accept as a contact.
pub fn subject_for(origin: &str) -> String {
    match origin.strip_prefix("https://") {
        Some(_) => origin.trim_end_matches('/').to_string(),
        None => {
            let host = origin
                .rsplit('/')
                .next()
                .unwrap_or("localhost")
                .split(':')
                .next()
                .unwrap_or("localhost");
            format!("mailto:kampr@{host}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_written_once_and_loaded_thereafter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("vapid.pem");
        let first = Vapid::load_or_create(&path, "mailto:x@y").unwrap();
        let again = Vapid::load_or_create(&path, "mailto:x@y").unwrap();
        assert_eq!(
            first.public_key_b64(),
            again.public_key_b64(),
            "rotating the key would invalidate every subscription already issued"
        );
    }

    /// An `applicationServerKey` is the raw uncompressed P-256 point, base64url without padding.
    /// A browser refuses anything else, and it refuses it at `subscribe()` — far from here.
    #[test]
    fn the_public_key_is_a_65_byte_uncompressed_point() {
        let dir = tempfile::tempdir().unwrap();
        let vapid = Vapid::load_or_create(&dir.path().join("vapid.pem"), "mailto:x@y").unwrap();
        let raw = URL_SAFE_NO_PAD.decode(vapid.public_key_b64()).unwrap();
        assert_eq!(raw.len(), 65);
        assert_eq!(raw[0], 0x04);
        assert!(!vapid.public_key_b64().contains('='));
    }

    #[test]
    #[cfg(unix)]
    fn the_private_key_is_no_more_readable_than_the_device_database() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("vapid.pem");
        Vapid::load_or_create(&path, "mailto:x@y").unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    #[test]
    fn a_junk_key_file_is_an_error_rather_than_a_node_that_pushes_nothing_silently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vapid.pem");
        std::fs::write(&path, "not a key").unwrap();
        assert!(matches!(
            Vapid::load(&path, "mailto:x@y"),
            Err(VapidError::Malformed(_, _))
        ));
    }

    #[test]
    fn the_subject_is_a_uri_a_push_service_will_accept() {
        assert_eq!(
            subject_for("https://kampr.example.com"),
            "https://kampr.example.com"
        );
        assert_eq!(
            subject_for("http://192.168.1.24:8790"),
            "mailto:kampr@192.168.1.24"
        );
        assert_eq!(subject_for("http://127.0.0.1:8790"), "mailto:kampr@127.0.0.1");
    }
}
