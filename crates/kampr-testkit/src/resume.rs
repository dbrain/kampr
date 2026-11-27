//! A program's half of `OSC 7577`: what a tool does with the token it finds in its environment.

use sha2::{Digest as _, Sha256};

/// `body` with `:mac=` appended: the SHA-256 of the body, a colon and the token, in lowercase hex.
pub fn sign(body: &str, token: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(body.as_bytes());
    hash.update(b":");
    hash.update(token.as_bytes());
    let mac: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{body}:mac={mac}")
}
