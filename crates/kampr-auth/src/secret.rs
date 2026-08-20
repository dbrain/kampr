use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// Codes are read off a screen and typed on a phone, so the alphabet excludes every pair a human
/// confuses: no `I`/`1`, no `O`/`0`, no `U` (it reads as `V` in a condensed font).
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTVWXYZ";
const CODE_LEN: usize = 8;

#[derive(Debug, thiserror::Error)]
#[error("the system random source failed: {0}")]
pub struct RandomError(#[from] getrandom::Error);

pub fn random_bytes(n: usize) -> Result<Vec<u8>, RandomError> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf)?;
    Ok(buf)
}

/// A bearer token. 256 bits of system entropy, so the stored digest needs no key stretching —
/// there is nothing to guess and nothing to dictionary-attack.
pub fn token() -> Result<String, RandomError> {
    Ok(format!("kmp_{}", URL_SAFE_NO_PAD.encode(random_bytes(32)?)))
}

/// A pairing code, printed as `XXXX-XXXX`. ~39 bits, which is *not* enough on its own — the
/// short TTL, the single use and the rate limiter are what make it safe.
pub fn pairing_code() -> Result<String, RandomError> {
    let raw = random_bytes(CODE_LEN)?;
    let chars: String = raw
        .iter()
        .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
        .collect();
    Ok(format!("{}-{}", &chars[..4], &chars[4..]))
}

/// Uppercases and strips anything that is not in the alphabet, so a code typed with the dash, in
/// lower case, or with a stray space still matches.
pub fn normalise_code(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_uppercase)
        .filter(|c| CODE_ALPHABET.contains(&(*c as u8)))
        .collect()
}

/// For a token, and only for a token: 256 bits of system entropy has no preimage worth
/// searching for, so the cheap hash is the right one.
pub fn digest(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// A pairing code is ~39.6 bits, which an offline attacker with a copy of the database walks
/// through in minutes against a bare hash. The lookup is by digest, so the salt has to be
/// constant — the defence is the work factor, and a code that lives ten minutes and dies on
/// first use never gives a table time to pay for itself.
const PAIRING_SALT: &[u8] = b"kampr/pairing/v1";

pub fn pairing_digest(normalised_code: &str) -> String {
    let params = Params::default();
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    match argon.hash_password_into(normalised_code.as_bytes(), PAIRING_SALT, &mut out) {
        Ok(()) => hex::encode(out),
        // Only reachable if the parameters are invalid, which they are not; falling back to the
        // cheap hash would silently reinstate the defect this exists to close.
        Err(e) => panic!("argon2 parameters are wrong: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_prefixed_and_url_safe() {
        let t = token().unwrap();
        assert!(t.starts_with("kmp_"));
        assert!(
            t[4..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(t, token().unwrap());
    }

    #[test]
    fn a_pairing_code_is_grouped_and_free_of_confusable_glyphs() {
        let code = pairing_code().unwrap();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        for c in code.chars().filter(|c| *c != '-') {
            assert!(CODE_ALPHABET.contains(&(c as u8)), "{c} is confusable");
        }
    }

    #[test]
    fn normalising_survives_how_a_human_types_it() {
        let code = pairing_code().unwrap();
        let bare = code.replace('-', "");
        for typed in [
            code.clone(),
            bare.clone(),
            code.to_lowercase(),
            format!(" {bare} "),
        ] {
            assert_eq!(normalise_code(&typed), bare);
        }
    }

    #[test]
    fn a_pairing_digest_is_stretched_rather_than_a_bare_hash() {
        // ~39.6 bits of entropy sits in a world-readable-until-now WAL; a bare SHA-256 of it is
        // an offline brute force with no rate limiter in the way.
        let code = pairing_code().unwrap();
        let normalised = normalise_code(&code);
        assert_ne!(pairing_digest(&normalised), digest(&normalised));
        assert_eq!(pairing_digest(&normalised), pairing_digest(&normalised));
        let at = std::time::Instant::now();
        pairing_digest(&normalised);
        assert!(
            at.elapsed() >= std::time::Duration::from_millis(10),
            "a pairing digest must cost real work, took {:?}",
            at.elapsed()
        );
    }

    #[test]
    fn digest_is_stable_and_not_the_secret() {
        assert_eq!(digest("kmp_abc"), digest("kmp_abc"));
        assert_ne!(digest("kmp_abc"), "kmp_abc");
        assert_eq!(digest("kmp_abc").len(), 64);
    }
}
