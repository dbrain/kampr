//! A container's detail read through a hub: the sheet's whole payload rides the peer's `managed`
//! ack, and a hub that rebuilt the ack from fields it knows would drop it.

use crate::mesh_files::Herd;
use kampr_auth::Role;
use kampr_node::manage::ManageOp;
use kampr_testkit::docker::{Container, available};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_container_detail_crosses_the_hub_whole() {
    if !available() {
        return;
    }
    let herd = Herd::start_with(Role::Full, |config| config.docker.enabled = Some(true)).await;
    let running = Container::run(&["-e", "KT_SEEN=yes", "--", "echo over-the-mesh; sleep 600"]);
    let raw = json!({ "t": "manage", "op": "docker.detail", "node": herd.peer_id(), "id": running.0 });
    let op: ManageOp = serde_json::from_value(raw.clone()).unwrap();

    let ack = kampr_node::docker::served(&herd.c.h.node, &op, raw)
        .await
        .expect("an ack from the peer");
    assert_eq!(ack["ok"], true, "{ack}");
    assert_eq!(ack["detail"]["name"], running.0.as_str(), "{ack}");
    assert!(
        ack["detail"]["env"]
            .as_array()
            .unwrap()
            .contains(&json!("KT_SEEN=yes")),
        "{ack}"
    );
    assert!(
        ack["detail"]["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["line"] == "over-the-mesh"),
        "{ack}"
    );
}
