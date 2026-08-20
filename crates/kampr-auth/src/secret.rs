use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// Codes are read off a screen and typed on a phone, so the alphabet excludes every pair a human
/// confuses: no `I`/`1`, no `O`/`0`, no `U` (it reads as `V` in a condensed font).
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTVWXYZ";
const CODE_LEN: usize = 8;

/// ~99 bits over the same alphabet, against the pairing code's ~39.6.
///
/// A pairing code is safe because it dies in ten minutes, dies on first use, and has a limiter
/// in front of it. A recovery code has none of that: it is written on paper, it is the only way
/// back into a host where a full-role device can run anything, and its digest sits in a database
/// an attacker may one day hold. At 20 characters an offline search is out of reach even against
/// a bare hash, which is what leaves the argon2 work factor as depth rather than as the defence.
const RECOVERY_LEN: usize = 20;

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
    Ok(grouped(&code_chars(CODE_LEN)?, 4))
}

/// A recovery code, printed as five groups of four. Shown once at `kampr init`, redeemed once,
/// and replaced by a fresh one the moment it is used.
pub fn recovery_code() -> Result<String, RandomError> {
    Ok(grouped(&code_chars(RECOVERY_LEN)?, 4))
}

/// Rejection sampling, not `% 31`: 256 is not a multiple of the alphabet, so the modulo favours
/// the first eight glyphs. The bias is small, and a credential whose whole defence is its
/// entropy should not carry any.
fn code_chars(len: usize) -> Result<String, RandomError> {
    let limit = 256 - 256 % CODE_ALPHABET.len();
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for byte in random_bytes(len * 2)? {
            if (byte as usize) < limit {
                out.push(CODE_ALPHABET[byte as usize % CODE_ALPHABET.len()] as char);
                if out.len() == len {
                    break;
                }
            }
        }
    }
    Ok(out)
}

fn grouped(chars: &str, group: usize) -> String {
    chars
        .as_bytes()
        .chunks(group)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-")
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

/// Same scheme, different domain: one table must never answer for both credential classes.
const RECOVERY_SALT: &[u8] = b"kampr/recovery/v1";

pub fn pairing_digest(normalised_code: &str) -> String {
    stretch(normalised_code, PAIRING_SALT)
}

pub fn recovery_digest(normalised_code: &str) -> String {
    stretch(normalised_code, RECOVERY_SALT)
}

fn stretch(normalised_code: &str, salt: &[u8]) -> String {
    let params = Params::default();
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    match argon.hash_password_into(normalised_code.as_bytes(), salt, &mut out) {
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

    #[test]
    fn a_recovery_code_is_far_stronger_than_a_pairing_code() {
        let code = recovery_code().unwrap();
        let bare = normalise_code(&code);
        assert_eq!(bare.len(), RECOVERY_LEN);
        // The pairing code survives on a ten-minute TTL, a single use and a limiter. A recovery
        // code has none of those, so the entropy has to carry it alone.
        let bits = |chars: usize| chars as f64 * (CODE_ALPHABET.len() as f64).log2();
        assert!(bits(RECOVERY_LEN) > 96.0, "{} bits", bits(RECOVERY_LEN));
        assert!(bits(RECOVERY_LEN) > 2.0 * bits(CODE_LEN));
        for c in code.chars().filter(|c| *c != '-') {
            assert!(CODE_ALPHABET.contains(&(c as u8)), "{c} is confusable");
        }
        assert_ne!(code, recovery_code().unwrap());
    }

    #[test]
    fn a_recovery_digest_is_stretched_and_not_the_pairing_digest_of_the_same_string() {
        let code = normalise_code(&recovery_code().unwrap());
        assert_ne!(recovery_digest(&code), digest(&code));
        // Domain separation: a table built against one credential class must not answer for the
        // other.
        assert_ne!(recovery_digest(&code), pairing_digest(&code));
        assert_eq!(recovery_digest(&code), recovery_digest(&code));
    }

    #[test]
    fn code_characters_are_drawn_without_modulo_bias() {
        // 256 % 31 != 0, so a plain `byte % 31` favours the first eight glyphs. It is a small
        // bias, and a credential that has to stand on its entropy alone should not carry it.
        let mut counts = [0usize; 31];
        for _ in 0..20_000 {
            for c in normalise_code(&recovery_code().unwrap()).bytes() {
                counts[CODE_ALPHABET.iter().position(|a| *a == c).unwrap()] += 1;
            }
        }
        let expected = (20_000 * RECOVERY_LEN) as f64 / 31.0;
        let chi: f64 = counts
            .iter()
            .map(|n| (*n as f64 - expected).powi(2) / expected)
            .sum();
        assert!(chi < 60.0, "chi-square {chi} over 30 degrees of freedom");
    }
}
