use crate::transport::{Incoming, Link, LinkError, Outgoing};
use kampr_auth::identity::{fingerprint_of, verify};
use kampr_auth::{MeshNode, MeshRole, NodeIdentity, Store};
use serde::{Deserialize, Serialize};

pub const MESH_PROTOCOL: u32 = 1;

/// Bytes of nonce each side contributes. Both go into the transcript, so a signature is only ever
/// valid for the one connection that produced both halves.
const NONCE_BYTES: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub node_id: String,
    pub node_name: String,
    pub build: String,
    /// Hex ed25519 verifying key.
    pub key: String,
    pub nonce: String,
    /// Present only on the connection that enrols this node, and spent by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub protocol: u32,
    pub node_id: String,
    pub node_name: String,
    pub build: String,
    pub key: String,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum PeerMsg {
    #[serde(rename = "mesh.hello")]
    Hello(Hello),
    #[serde(rename = "mesh.auth")]
    Auth { sig: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum HubMsg {
    #[serde(rename = "mesh.challenge")]
    Challenge(Challenge),
    #[serde(rename = "mesh.accepted")]
    Accepted { sig: String, enrolled: bool },
    #[serde(rename = "mesh.refused")]
    Refused { code: String, reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("{reason}")]
    Refused { code: String, reason: String },
    #[error(transparent)]
    Link(#[from] LinkError),
    #[error("{0}")]
    Protocol(String),
    #[error(transparent)]
    Store(#[from] kampr_auth::StoreError),
}

impl HandshakeError {
    fn refused(code: &str, reason: &str) -> Self {
        Self::Refused {
            code: code.into(),
            reason: reason.into(),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Refused { code, .. } => code,
            Self::Link(_) => "closed",
            Self::Protocol(_) => "protocol",
            Self::Store(_) => "unavailable",
        }
    }
}

/// What the hub learned about the node that dialled in.
#[derive(Debug, Clone)]
pub struct Accepted {
    pub node: MeshNode,
    pub build: String,
    /// True when this connection is the one that spent a join code — worth an audit line and a
    /// console notice, and nothing else.
    pub enrolled: bool,
}

/// What the peer learned about the hub it dialled.
#[derive(Debug, Clone)]
pub struct HubIdentity {
    pub node_id: String,
    pub node_name: String,
    pub build: String,
    pub key: String,
}

impl HubIdentity {
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.key)
    }
}

/// Everything a node needs to present itself, in either direction.
#[derive(Debug, Clone)]
pub struct Presence {
    pub node_id: String,
    pub node_name: String,
    pub build: String,
}

/// Everything both signatures cover.
///
/// Both public keys, both nonces and both node ids, under a version label. Binding the keys is
/// what stops a signature being replayed against a third node; binding both nonces is what stops
/// it being replayed against the same one; binding the version is what stops a later protocol
/// being negotiated down to this one. The role prefix is separate domain separation, so a
/// signature made as a peer can never be presented as a hub's.
pub fn transcript(
    hub_key: &str,
    peer_key: &str,
    hub_nonce: &str,
    peer_nonce: &str,
    hub_id: &str,
    peer_id: &str,
) -> String {
    format!(
        "KAMPR-MESH/{MESH_PROTOCOL}\n\
         hub-key={hub_key}\n\
         peer-key={peer_key}\n\
         hub-nonce={hub_nonce}\n\
         peer-nonce={peer_nonce}\n\
         hub-node={hub_id}\n\
         peer-node={peer_id}\n"
    )
}

fn as_peer(transcript: &str) -> Vec<u8> {
    format!("peer\n{transcript}").into_bytes()
}

fn as_hub(transcript: &str) -> Vec<u8> {
    format!("hub\n{transcript}").into_bytes()
}

fn nonce() -> Result<String, HandshakeError> {
    kampr_auth::secret::random_bytes(NONCE_BYTES)
        .map(hex::encode)
        .map_err(|e| HandshakeError::Protocol(e.to_string()))
}

/// The hub's half.
///
/// Verifies the peer's signature *before* looking at enrolment, and proves its own key only after
/// it has decided to accept — so a stranger neither learns whether a key would have been accepted
/// nor collects a hub signature over a transcript it chose.
pub async fn accept<O: Outgoing, I: Incoming>(
    link: &mut Link<O, I>,
    identity: &NodeIdentity,
    me: &Presence,
    store: &Store,
    now: i64,
) -> Result<Accepted, HandshakeError> {
    let hello = match link.recv::<PeerMsg>().await? {
        PeerMsg::Hello(hello) => hello,
        PeerMsg::Auth { .. } => {
            return refuse(link, "protocol", "a mesh link opens with mesh.hello").await;
        }
    };
    if hello.protocol != MESH_PROTOCOL {
        return refuse(
            link,
            "protocol",
            &format!("this hub speaks mesh protocol {MESH_PROTOCOL}"),
        )
        .await;
    }
    if hex::decode(&hello.key).map(|k| k.len()) != Ok(32) {
        return refuse(link, "protocol", "a node key is 32 bytes of hex").await;
    }

    let my_nonce = nonce()?;
    link.send(&HubMsg::Challenge(Challenge {
        protocol: MESH_PROTOCOL,
        node_id: me.node_id.clone(),
        node_name: me.node_name.clone(),
        build: me.build.clone(),
        key: identity.public_hex(),
        nonce: my_nonce.clone(),
    }))
    .await?;

    let PeerMsg::Auth { sig } = link.recv::<PeerMsg>().await? else {
        return refuse(link, "protocol", "expected mesh.auth").await;
    };

    let transcript = transcript(
        &identity.public_hex(),
        &hello.key,
        &my_nonce,
        &hello.nonce,
        &me.node_id,
        &hello.node_id,
    );
    if !verify(&hello.key, &as_peer(&transcript), &sig) {
        return refuse(link, "bad_signature", "that key did not sign this handshake").await;
    }

    let mesh = store.mesh();
    let known = mesh.node(&hello.key).await?;
    let mut enrolled = false;
    if !known.as_ref().is_some_and(MeshNode::active) {
        let claimed = match hello.join.as_deref() {
            Some(code) => mesh.claim_invite(code, now).await?,
            None => false,
        };
        if !claimed {
            let (code, reason) = match known {
                Some(_) => ("revoked", "this node has been revoked from the herd"),
                None => (
                    "unenrolled",
                    "this node is not enrolled here; ask the hub operator for a join code",
                ),
            };
            return refuse(link, code, reason).await;
        }
        enrolled = true;
    }
    let node = mesh
        .enrol(
            &hello.key,
            &hello.node_id,
            &hello.node_name,
            MeshRole::Peer,
            None,
            now,
        )
        .await?;
    mesh.touch(&hello.key, now).await?;

    link.send(&HubMsg::Accepted {
        sig: identity.sign(&as_hub(&transcript)),
        enrolled,
    })
    .await?;
    Ok(Accepted {
        node,
        build: hello.build,
        enrolled,
    })
}

/// The peer's half. Refuses to serve anything until the hub has proved itself against the key
/// this node pinned when it joined — an operator who confirmed a fingerprint once is not asked to
/// trust the address again.
pub async fn greet<O: Outgoing, I: Incoming>(
    link: &mut Link<O, I>,
    identity: &NodeIdentity,
    me: &Presence,
    join: Option<&str>,
    expect: Option<&str>,
) -> Result<HubIdentity, HandshakeError> {
    let my_nonce = nonce()?;
    link.send(&PeerMsg::Hello(Hello {
        protocol: MESH_PROTOCOL,
        node_id: me.node_id.clone(),
        node_name: me.node_name.clone(),
        build: me.build.clone(),
        key: identity.public_hex(),
        nonce: my_nonce.clone(),
        join: join.map(str::to_string),
    }))
    .await?;

    let challenge = match link.recv::<HubMsg>().await? {
        HubMsg::Challenge(challenge) => challenge,
        HubMsg::Refused { code, reason } => return Err(HandshakeError::Refused { code, reason }),
        HubMsg::Accepted { .. } => {
            return Err(HandshakeError::Protocol(
                "the hub accepted a handshake this node never made".into(),
            ));
        }
    };
    if challenge.protocol != MESH_PROTOCOL {
        return Err(HandshakeError::Protocol(format!(
            "the hub speaks mesh protocol {}, this node speaks {MESH_PROTOCOL}",
            challenge.protocol
        )));
    }
    if let Some(expected) = expect
        && expected != challenge.key
    {
        return Err(HandshakeError::refused(
            "wrong_hub",
            &format!(
                "the node answering there has key {} — expected {}",
                fingerprint_of(&challenge.key),
                fingerprint_of(expected)
            ),
        ));
    }

    let transcript = transcript(
        &challenge.key,
        &identity.public_hex(),
        &challenge.nonce,
        &my_nonce,
        &challenge.node_id,
        &me.node_id,
    );
    link.send(&PeerMsg::Auth {
        sig: identity.sign(&as_peer(&transcript)),
    })
    .await?;

    match link.recv::<HubMsg>().await? {
        HubMsg::Accepted { sig, .. } => {
            if !verify(&challenge.key, &as_hub(&transcript), &sig) {
                return Err(HandshakeError::refused(
                    "wrong_hub",
                    "the hub did not sign this handshake with the key it presented",
                ));
            }
            Ok(HubIdentity {
                node_id: challenge.node_id,
                node_name: challenge.node_name,
                build: challenge.build,
                key: challenge.key,
            })
        }
        HubMsg::Refused { code, reason } => Err(HandshakeError::Refused { code, reason }),
        HubMsg::Challenge(_) => Err(HandshakeError::Protocol("a second challenge".into())),
    }
}

async fn refuse<O: Outgoing, I: Incoming>(
    link: &mut Link<O, I>,
    code: &str,
    reason: &str,
) -> Result<Accepted, HandshakeError> {
    let _ = link
        .send(&HubMsg::Refused {
            code: code.into(),
            reason: reason.into(),
        })
        .await;
    link.out.close().await;
    Err(HandshakeError::refused(code, reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::pair;

    struct Node {
        identity: NodeIdentity,
        me: Presence,
        _dir: tempfile::TempDir,
    }

    fn node(name: &str) -> Node {
        let dir = tempfile::tempdir().unwrap();
        Node {
            identity: NodeIdentity::load_or_create(&dir.path().join("node.key")).unwrap(),
            me: Presence {
                node_id: format!("01J{name}"),
                node_name: name.to_string(),
                build: "0.1.0".into(),
            },
            _dir: dir,
        }
    }

    async fn store() -> Store {
        Store::open_memory().await.unwrap()
    }

    /// Runs both halves against each other and hands back what each concluded.
    async fn shake(
        hub: &Node,
        peer: &Node,
        store: &Store,
        join: Option<String>,
        expect: Option<String>,
    ) -> (
        Result<Accepted, HandshakeError>,
        Result<HubIdentity, HandshakeError>,
    ) {
        let (mut a, mut b) = pair();
        let hub_side = accept(&mut a, &hub.identity, &hub.me, store, 1_000);
        let peer_side = greet(
            &mut b,
            &peer.identity,
            &peer.me,
            join.as_deref(),
            expect.as_deref(),
        );
        tokio::join!(hub_side, peer_side)
    }

    #[tokio::test]
    async fn a_join_code_enrols_the_peer_and_both_ends_learn_the_other() {
        let (hub, peer, store) = (node("hub"), node("laptop"), store().await);
        let code = store.mesh().invite(1, 10_000).await.unwrap();
        let (accepted, joined) = shake(&hub, &peer, &store, Some(code), None).await;

        let accepted = accepted.unwrap();
        assert!(accepted.enrolled);
        assert_eq!(accepted.node.pubkey, peer.identity.public_hex());
        assert_eq!(accepted.node.name, "laptop");

        let joined = joined.unwrap();
        assert_eq!(joined.key, hub.identity.public_hex());
        assert_eq!(joined.fingerprint(), hub.identity.fingerprint());
    }

    #[tokio::test]
    async fn an_unenrolled_node_is_refused_and_told_why() {
        let (hub, peer, store) = (node("hub"), node("stranger"), store().await);
        let (accepted, joined) = shake(&hub, &peer, &store, None, None).await;
        assert_eq!(accepted.unwrap_err().code(), "unenrolled");
        assert_eq!(joined.unwrap_err().code(), "unenrolled");
        assert!(
            store
                .mesh()
                .node(&peer.identity.public_hex())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_wrong_join_code_enrols_nobody() {
        let (hub, peer, store) = (node("hub"), node("stranger"), store().await);
        store.mesh().invite(1, 10_000).await.unwrap();
        let (accepted, _) = shake(&hub, &peer, &store, Some("ZZZZ-ZZZZ".into()), None).await;
        assert_eq!(accepted.unwrap_err().code(), "unenrolled");
    }

    #[tokio::test]
    async fn an_enrolled_node_reconnects_with_no_code_at_all() {
        let (hub, peer, store) = (node("hub"), node("laptop"), store().await);
        let code = store.mesh().invite(1, 10_000).await.unwrap();
        shake(&hub, &peer, &store, Some(code), None).await.0.unwrap();

        let (accepted, _) = shake(&hub, &peer, &store, None, None).await;
        let accepted = accepted.unwrap();
        assert!(!accepted.enrolled, "the second connection enrols nothing");
    }

    #[tokio::test]
    async fn a_revoked_node_is_refused_even_though_its_key_is_known() {
        let (hub, peer, store) = (node("hub"), node("laptop"), store().await);
        let code = store.mesh().invite(1, 10_000).await.unwrap();
        shake(&hub, &peer, &store, Some(code), None).await.0.unwrap();
        store.mesh().revoke("laptop", 2_000).await.unwrap();

        let (accepted, joined) = shake(&hub, &peer, &store, None, None).await;
        assert_eq!(accepted.unwrap_err().code(), "revoked");
        assert_eq!(joined.unwrap_err().code(), "revoked");
    }

    #[tokio::test]
    async fn a_stolen_signature_does_not_authenticate_the_thief() {
        let (hub, peer, thief, store) = (node("hub"), node("laptop"), node("thief"), store().await);
        let code = store.mesh().invite(1, 10_000).await.unwrap();
        shake(&hub, &peer, &store, Some(code), None).await.0.unwrap();

        // The thief presents the enrolled node's key with its own private half — the only thing a
        // key on a wire is worth on its own.
        let (mut a, mut b) = pair();
        let hub_side = accept(&mut a, &hub.identity, &hub.me, &store, 3_000);
        let forged = async {
            b.send(&PeerMsg::Hello(Hello {
                protocol: MESH_PROTOCOL,
                node_id: peer.me.node_id.clone(),
                node_name: peer.me.node_name.clone(),
                build: "0.1.0".into(),
                key: peer.identity.public_hex(),
                nonce: "00".repeat(32),
                join: None,
            }))
            .await
            .unwrap();
            let HubMsg::Challenge(challenge) = b.recv::<HubMsg>().await.unwrap() else {
                panic!("expected a challenge");
            };
            let transcript = transcript(
                &challenge.key,
                &peer.identity.public_hex(),
                &challenge.nonce,
                &"00".repeat(32),
                &challenge.node_id,
                &peer.me.node_id,
            );
            b.send(&PeerMsg::Auth {
                sig: thief.identity.sign(&as_peer(&transcript)),
            })
            .await
            .unwrap();
        };
        let (accepted, ()) = tokio::join!(hub_side, forged);
        assert_eq!(accepted.unwrap_err().code(), "bad_signature");
    }

    #[tokio::test]
    async fn a_pinned_peer_refuses_a_hub_that_is_not_the_one_it_joined() {
        let (hub, impostor, peer) = (node("hub"), node("impostor"), node("laptop"));
        let store = store().await;
        let code = store.mesh().invite(1, 10_000).await.unwrap();
        let (_, joined) = shake(&hub, &peer, &store, Some(code), None).await;
        let pin = joined.unwrap().key;

        let elsewhere = Store::open_memory().await.unwrap();
        elsewhere
            .mesh()
            .enrol(
                &peer.identity.public_hex(),
                &peer.me.node_id,
                "laptop",
                MeshRole::Peer,
                None,
                1,
            )
            .await
            .unwrap();
        let (_, joined) = shake(&impostor, &peer, &elsewhere, None, Some(pin)).await;
        assert_eq!(joined.unwrap_err().code(), "wrong_hub");
    }

    #[tokio::test]
    async fn a_hub_signature_from_the_wrong_key_is_refused_after_acceptance() {
        let (hub, other, peer) = (node("hub"), node("other"), node("laptop"));
        let (mut a, mut b) = pair();
        let peer_side = greet(&mut b, &peer.identity, &peer.me, None, None);
        let liar = async {
            let PeerMsg::Hello(hello) = a.recv::<PeerMsg>().await.unwrap() else {
                panic!("expected hello");
            };
            let nonce = "11".repeat(32);
            a.send(&HubMsg::Challenge(Challenge {
                protocol: MESH_PROTOCOL,
                node_id: hub.me.node_id.clone(),
                node_name: "hub".into(),
                build: "0.1.0".into(),
                key: hub.identity.public_hex(),
                nonce: nonce.clone(),
            }))
            .await
            .unwrap();
            let PeerMsg::Auth { .. } = a.recv::<PeerMsg>().await.unwrap() else {
                panic!("expected auth");
            };
            let transcript = transcript(
                &hub.identity.public_hex(),
                &hello.key,
                &nonce,
                &hello.nonce,
                &hub.me.node_id,
                &hello.node_id,
            );
            // Signed with a key it never presented.
            a.send(&HubMsg::Accepted {
                sig: other.identity.sign(&as_hub(&transcript)),
                enrolled: false,
            })
            .await
            .unwrap();
        };
        let (joined, ()) = tokio::join!(peer_side, liar);
        assert_eq!(joined.unwrap_err().code(), "wrong_hub");
    }

    #[tokio::test]
    async fn a_signature_from_one_connection_does_not_open_the_next() {
        let (hub, peer, store) = (node("hub"), node("laptop"), store().await);
        let code = store.mesh().invite(1, 10_000).await.unwrap();
        shake(&hub, &peer, &store, Some(code), None).await.0.unwrap();

        // Replay: the same peer hello and the same signature, against a fresh hub nonce.
        let (mut a, mut b) = pair();
        let hub_side = accept(&mut a, &hub.identity, &hub.me, &store, 4_000);
        let replay = async {
            let stale = "22".repeat(32);
            b.send(&PeerMsg::Hello(Hello {
                protocol: MESH_PROTOCOL,
                node_id: peer.me.node_id.clone(),
                node_name: "laptop".into(),
                build: "0.1.0".into(),
                key: peer.identity.public_hex(),
                nonce: stale.clone(),
                join: None,
            }))
            .await
            .unwrap();
            let HubMsg::Challenge(_) = b.recv::<HubMsg>().await.unwrap() else {
                panic!("expected a challenge");
            };
            let transcript = transcript(
                &hub.identity.public_hex(),
                &peer.identity.public_hex(),
                // A nonce from some earlier connection rather than this one's.
                &"33".repeat(32),
                &stale,
                &hub.me.node_id,
                &peer.me.node_id,
            );
            b.send(&PeerMsg::Auth {
                sig: peer.identity.sign(&as_peer(&transcript)),
            })
            .await
            .unwrap();
        };
        let (accepted, ()) = tokio::join!(hub_side, replay);
        assert_eq!(accepted.unwrap_err().code(), "bad_signature");
    }

    #[tokio::test]
    async fn a_peer_signature_cannot_be_presented_as_a_hubs() {
        let identity = node("x").identity;
        let transcript = transcript("a", "b", "c", "d", "e", "f");
        let signature = identity.sign(&as_peer(&transcript));
        assert!(verify(&identity.public_hex(), &as_peer(&transcript), &signature));
        assert!(!verify(&identity.public_hex(), &as_hub(&transcript), &signature));
    }

    #[tokio::test]
    async fn a_version_this_hub_does_not_speak_is_refused_before_anything_else() {
        let (hub, store) = (node("hub"), store().await);
        let (mut a, mut b) = pair();
        let hub_side = accept(&mut a, &hub.identity, &hub.me, &store, 1_000);
        let future = async {
            b.send(&PeerMsg::Hello(Hello {
                protocol: MESH_PROTOCOL + 1,
                node_id: "01JX".into(),
                node_name: "future".into(),
                build: "9.9.9".into(),
                key: "00".repeat(32),
                nonce: "00".repeat(32),
                join: None,
            }))
            .await
            .unwrap();
            b.recv::<HubMsg>().await.unwrap()
        };
        let (accepted, verdict) = tokio::join!(hub_side, future);
        assert_eq!(accepted.unwrap_err().code(), "protocol");
        assert!(matches!(verdict, HubMsg::Refused { .. }));
    }
}
