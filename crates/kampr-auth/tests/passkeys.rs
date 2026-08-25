//! What registering a passkey does to the device list, driven through a real ceremony.
//!
//! There is no soft authenticator in this workspace and adding one is a dependency. A `none`
//! attestation carries no signature to verify, so the authenticator's half of a *registration* is
//! a CBOR document and a public key that is genuinely on the curve — OpenSSL is asked, so a
//! made-up coordinate pair is refused. That is all this builds, and it is enough to reach the
//! code under test the way a phone reaches it.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use kampr_auth::{
    AuditLog, Auth, AuthError, Client, Delivery, Device, Enrolment, Policy, Role, Store, Tier, now,
};
use sha2::{Digest, Sha256};
use webauthn_rs::prelude::RegisterPublicKeyCredential;

const ORIGIN: &str = "https://kampr.example.com";
const PEER: &str = "1.2.3.4";

/// The P-256 generator. Any point will do provided it is really on the curve, and this one is by
/// definition; nothing here ever signs with it.
fn cose_key() -> Vec<u8> {
    let x = hex::decode("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296").unwrap();
    let y = hex::decode("4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    let mut out = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
    out.extend_from_slice(&x);
    out.extend_from_slice(&[0x22, 0x58, 0x20]);
    out.extend_from_slice(&y);
    out
}

fn authenticator_data(rp_id: &str, cred_id: &[u8]) -> Vec<u8> {
    let mut out = Sha256::digest(rp_id.as_bytes()).to_vec();
    // User present, user verified, attested credential data. Backup eligibility stays clear:
    // webauthn-rs refuses a credential that claims to be backed up without it.
    out.push(0x01 | 0x04 | 0x40);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
    out.extend_from_slice(cred_id);
    out.extend_from_slice(&cose_key());
    out
}

fn attestation_object(auth_data: &[u8]) -> Vec<u8> {
    assert!(
        auth_data.len() < 256,
        "the short byte-string header only covers 255"
    );
    let mut out = vec![0xa3];
    out.extend_from_slice(b"\x63fmt\x64none");
    out.extend_from_slice(b"\x67attStmt\xa0");
    out.extend_from_slice(b"\x68authData\x58");
    out.push(auth_data.len() as u8);
    out.extend_from_slice(auth_data);
    out
}

fn registration(rp_id: &str, challenge: &[u8], cred_id: &[u8]) -> RegisterPublicKeyCredential {
    let client_data = serde_json::json!({
        "type": "webauthn.create",
        "challenge": URL_SAFE_NO_PAD.encode(challenge),
        "origin": ORIGIN,
        "crossOrigin": false,
    })
    .to_string();
    let object = attestation_object(&authenticator_data(rp_id, cred_id));
    serde_json::from_value(serde_json::json!({
        "id": URL_SAFE_NO_PAD.encode(cred_id),
        "rawId": URL_SAFE_NO_PAD.encode(cred_id),
        "type": "public-key",
        "response": {
            "attestationObject": URL_SAFE_NO_PAD.encode(&object),
            "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data.as_bytes()),
        },
    }))
    .expect("a registration response")
}

async fn tier_one() -> Auth {
    Auth::new(
        Store::open_memory().await.unwrap(),
        Tier::detect(ORIGIN).unwrap(),
        AuditLog::disabled(),
        Policy::default(),
        &[],
    )
    .unwrap()
}

async fn paired(a: &Auth, name: &str) -> Enrolment {
    let pairing = a.create_pairing(Role::Full, Delivery::Console).await.unwrap();
    if !pairing.armed {
        assert!(a.arm_pairing(&pairing.code).await.unwrap());
    }
    a.redeem_pairing(&pairing.code, name, None, PEER).await.unwrap()
}

async fn register(a: &Auth, device: &Device, cred_id: &[u8]) -> Enrolment {
    let (challenge_id, options) = a
        .start_passkey_registration(&device.name, Client::Browser)
        .await
        .unwrap();
    let rp_id = a
        .passkeys()
        .expect("a passkey engine at tier 1")
        .rp_id()
        .to_string();
    let credential = registration(&rp_id, options.public_key.challenge.as_ref(), cred_id);
    a.finish_passkey_registration(&challenge_id, &credential, device)
        .await
        .expect("a registration the node accepts")
}

/// A passkey enrolled by a device that is already in the list belongs to *that* device. Minting a
/// second row left the operator with two `full` devices of the same name, each holding its own
/// never-expiring token, and revoking the one he could see revoked half of it.
#[tokio::test]
async fn registering_a_passkey_binds_it_to_the_device_that_asked_for_it() {
    let a = tier_one().await;
    let phone = paired(&a, "phone").await;

    let bound = register(&a, &phone.device, b"credential-0001").await;

    let devices = a.devices().await.unwrap();
    assert_eq!(
        devices.len(),
        1,
        "a passkey is a second credential for one device, not a second device: {:?}",
        devices
            .iter()
            .map(|d| (&d.name, d.role.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(bound.device.id, phone.device.id);
    assert_eq!(
        a.authenticate(&phone.token, PEER).await.unwrap().id,
        phone.device.id,
        "the token the phone was already holding is still the token it holds"
    );
}

/// The property the duplicate row silently broke. An operator revokes the device he can see; if
/// the passkey hangs off a row he cannot, the phone signs straight back in.
#[tokio::test]
async fn revoking_the_device_that_registered_a_passkey_takes_the_passkey_with_it() {
    let a = tier_one().await;
    let phone = paired(&a, "phone").await;
    let bound = register(&a, &phone.device, b"credential-0001").await;
    let rp_id = a.passkeys().unwrap().rp_id().to_string();
    assert_eq!(a.store().credentials(&rp_id, now()).await.unwrap().len(), 1);

    assert!(a.revoke(&phone.device.id, &phone.device).await.unwrap());

    assert!(
        a.store().credentials(&rp_id, now()).await.unwrap().is_empty(),
        "the passkey outlived the revocation of the device that enrolled it"
    );
    assert!(matches!(
        a.start_passkey_authentication(PEER).await,
        Err(AuthError::UnknownCredential)
    ));
    for token in [&phone.token, &bound.token] {
        assert!(
            matches!(a.authenticate(token, PEER).await, Err(AuthError::Unauthorized)),
            "a token survived the revocation of its device"
        );
    }
}
