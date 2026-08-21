use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// What an Android app's WebAuthn ceremony calls itself.
///
/// Credential Manager does not send an `https://` origin the way a browser does. It sends
/// `android:apk-key-hash:<base64url of the SHA-256 of the calling app's signing certificate>`,
/// having first checked that the relying party's `/.well-known/assetlinks.json` delegates
/// `common.get_login_creds` to that package and that certificate. A relying party that does not
/// accept this origin refuses every native registration *after* the user has already approved it.
pub fn credential_manager_origin(fingerprint: &str) -> Option<String> {
    let hex = canonical_fingerprint(fingerprint)?;
    let bytes: Vec<u8> = hex
        .split(':')
        .filter_map(|byte| u8::from_str_radix(byte, 16).ok())
        .collect();
    Some(format!("android:apk-key-hash:{}", URL_SAFE_NO_PAD.encode(bytes)))
}

/// `keytool`, `apksigner` and a paste off a web page disagree about case and colons for the same
/// certificate. One spelling comes out: upper-case, colon-separated, 32 bytes.
pub fn canonical_fingerprint(fingerprint: &str) -> Option<String> {
    let hex: String = fingerprint
        .trim()
        .chars()
        .filter(|c| *c != ':')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(
        hex.as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).into_owned())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    /// Not a vector this file produced: the fingerprint and the origin below are the worked
    /// example in Corbado's write-up of native-app origin validation, which is a party with no
    /// stake in this implementation being right.
    const THEIR_FINGERPRINT: &str =
        "8B:BF:39:60:61:89:30:A4:45:F3:D7:09:1E:7B:1B:05:0F:8A:FD:AF:24:EB:F1:EB:2E:3D:13:88:09:FC:79:59";
    const THEIR_ORIGIN: &str = "android:apk-key-hash:i785YGGJMKRF89cJHnsbBQ-K_a8k6_HrLj0TiAn8eVk";

    #[test]
    fn the_origin_matches_one_somebody_else_worked_out() {
        assert_eq!(
            credential_manager_origin(THEIR_FINGERPRINT).as_deref(),
            Some(THEIR_ORIGIN)
        );
    }

    /// `webauthn-rs` compares an opaque origin by exact equality after parsing both sides as a
    /// URL. An origin that does not survive that round trip is one no ceremony can ever match.
    #[test]
    fn it_survives_being_parsed_as_a_url() {
        let origin = credential_manager_origin(THEIR_FINGERPRINT).expect("an origin");
        let parsed = Url::parse(&origin).expect("a url");
        assert_eq!(parsed.as_str(), origin);
        assert_eq!(parsed, Url::parse(THEIR_ORIGIN).expect("their url"));
    }

    #[test]
    fn the_same_certificate_typed_three_ways_is_one_origin() {
        let bare = THEIR_FINGERPRINT.replace(':', "").to_lowercase();
        assert_eq!(credential_manager_origin(&bare).as_deref(), Some(THEIR_ORIGIN));
        assert_eq!(
            credential_manager_origin(&format!("  {THEIR_FINGERPRINT}  ")).as_deref(),
            Some(THEIR_ORIGIN)
        );
    }

    #[test]
    fn anything_that_is_not_a_certificate_digest_yields_no_origin() {
        for junk in [
            "",
            "not-a-fingerprint",
            "AA:BB",
            &"AB".repeat(31),
            &"ZZ".repeat(32),
        ] {
            assert_eq!(credential_manager_origin(junk), None, "{junk}");
            assert_eq!(canonical_fingerprint(junk), None, "{junk}");
        }
    }
}
