use crate::config::Android;
use kampr_auth::android::canonical_fingerprint;

/// The SHA-256 of the certificate `~/.android-keystores/kampr-release.jks` holds — the key every
/// release APK is signed with, and therefore the app identity a phone presents to this node.
///
/// It is a public value: it is derivable from any copy of the APK. It is a default rather than a
/// hard-coded answer because two other builds exist and neither is signed with it — a debug build
/// is signed with the machine's own `~/.android/debug.keystore`, and anyone building Kampr from
/// source signs with a keystore of their own. Both put their fingerprint in `[android]` and this
/// file names their app instead. `assetlinks.rs`'s test reads it back out of the artefact rather
/// than trusting this line.
pub const RELEASE_FINGERPRINT: &str =
    "A0:8A:21:84:46:AA:2B:99:08:5C:67:0B:5A:9B:70:32:5E:05:F9:27:CC:DD:12:17:E7:94:63:13:C7:7F:C6:18";

/// The Digital Asset Links statement list, or `None` when this node delegates to no app.
///
/// Built once, by the caller that builds the router, because an unauthenticated endpoint that does
/// work per request is a way to make a node busy for free.
pub fn document(android: &Android) -> Option<String> {
    // The same certificate copied out of two tools is one certificate, wherever the two copies
    // ended up in the list.
    let mut seen = std::collections::HashSet::new();
    let fingerprints: Vec<String> = android
        .fingerprints
        .iter()
        .filter_map(|f| canonical_fingerprint(f))
        .filter(|f| seen.insert(f.clone()))
        .collect();
    if fingerprints.is_empty() || android.package_name.trim().is_empty() {
        return None;
    }
    Some(
        serde_json::json!([{
            // `common.get_login_creds` and nothing else. `common.handle_all_urls` is the app-link
            // relation, and Kampr claims no URL: an app link names its hosts in the manifest at
            // build time, and every operator's node is at a different one.
            "relation": ["delegate_permission/common.get_login_creds"],
            "target": {
                "namespace": "android_app",
                "package_name": android.package_name.trim(),
                "sha256_cert_fingerprints": fingerprints,
            },
        }])
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_default_is_a_well_formed_digest() {
        assert_eq!(
            canonical_fingerprint(RELEASE_FINGERPRINT).as_deref(),
            Some(RELEASE_FINGERPRINT)
        );
    }

    #[test]
    fn one_certificate_is_one_entry_wherever_its_copies_landed() {
        let android = Android {
            package_name: "dev.kampr.app".into(),
            fingerprints: vec![
                RELEASE_FINGERPRINT.into(),
                "AB".repeat(32),
                RELEASE_FINGERPRINT.to_lowercase(),
            ],
        };
        let document: serde_json::Value =
            serde_json::from_str(&document(&android).expect("a document")).expect("json");
        let fingerprints = document[0]["target"]["sha256_cert_fingerprints"]
            .as_array()
            .expect("fingerprints");
        assert_eq!(fingerprints.len(), 2, "{fingerprints:?}");
        assert_eq!(fingerprints[0], RELEASE_FINGERPRINT);
    }

    #[test]
    fn a_document_naming_no_usable_certificate_is_no_document() {
        for junk in ["", "not-a-fingerprint", "AA:BB", &"AB".repeat(31)] {
            let android = Android {
                package_name: "dev.kampr.app".into(),
                fingerprints: vec![junk.to_string()],
            };
            assert_eq!(document(&android), None, "{junk} is not a certificate digest");
        }
    }
}
