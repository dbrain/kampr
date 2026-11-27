//! `OSC 7577` over the wire (docs/25): what a boot left is listed by `native.resumable` and started
//! by `native.resume`, once.

use super::*;

const REPORT: &str = r#"
m=$(printf '%s:%s' "pid=$$:time=$(date +%s):argv=$ARGV" "$OSC7577_TOKEN" | sha256sum | cut -d' ' -f1)
printf '\033]7577;pid=%s:time=%s:argv=%s:mac=%s\033\\' $$ "$(date +%s)" "$ARGV" "$m"
exec sleep 600
"#;

/// A node restarted after a boot that took the keeper of a session whose program registered
/// `sh -c "echo brought-back; …"` under the label `work`, and that record as the node lists it.
async fn left_by_a_boot() -> (Harness, Client, Value) {
    let mut h = Harness::start().await;
    let mut client = h.client().await;
    let argv = ["sh", "-c", "echo brought-back; exec sleep 600"]
        .map(|a| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(a)
        })
        .join(",");
    let script = REPORT.replace("$ARGV", &argv);
    let id = client
        .create(&["sh", "-c", &script], json!({ "label": "work" }))
        .await;
    let state = h.home.as_ref().unwrap().path().join("state/resume");
    let local = id.rsplit_once("/n:").unwrap().1.to_string();
    let record = state.join(format!("{local}.json"));
    let deadline = std::time::Instant::now() + SOON;
    while !record.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "no record at {}",
            record.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    drop(client);

    let root = h.root();
    kampr_testkit::stop_keepers(&root);
    std::fs::remove_dir_all(root.join(&local)).unwrap();
    h = h.restart().await;
    let mut client = h.client().await;
    let listed = client
        .manage(json!({ "op": "native.resumable", "node": NODE }))
        .await;
    let records = listed["resumable"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .clone();
    assert_eq!(records.len(), 1, "{listed}");
    (h, client, records[0].clone())
}

#[tokio::test]
async fn a_command_a_program_registered_is_offered_after_a_boot_and_runs_once() {
    let (_h, mut client, record) = left_by_a_boot().await;
    assert_eq!(
        record["argv"],
        json!(["sh", "-c", "echo brought-back; exec sleep 600"])
    );
    assert_eq!(record["label"], "work");

    let ran = client
        .manage(json!({ "op": "native.resume", "node": NODE, "id": record["id"] }))
        .await;
    assert_eq!(ran["ok"], true, "{ran}");
    let pane = ran["id"].as_str().expect("the new pane").to_string();
    client.watch(&pane, false).await;
    client.shows(&pane, "brought-back").await;

    let again = client
        .manage(json!({ "op": "native.resumable", "node": NODE }))
        .await;
    assert_eq!(again["resumable"], json!([]), "{again}");
    let twice = client
        .manage(json!({ "op": "native.resume", "node": NODE, "id": record["id"] }))
        .await;
    assert_ne!(twice["ok"], true, "a record ran twice: {twice}");
}

#[tokio::test]
async fn a_dismissed_record_is_neither_offered_again_nor_run() {
    let (_h, mut client, record) = left_by_a_boot().await;
    let dismissed = client
        .manage(json!({ "op": "native.resume.dismiss", "node": NODE, "id": record["id"] }))
        .await;
    assert_eq!(dismissed["ok"], true, "{dismissed}");
    assert!(
        dismissed.get("id").is_none(),
        "a dismissal made a pane: {dismissed}"
    );

    let again = client
        .manage(json!({ "op": "native.resumable", "node": NODE }))
        .await;
    assert_eq!(again["resumable"], json!([]), "{again}");
    let ran = client
        .manage(json!({ "op": "native.resume", "node": NODE, "id": record["id"] }))
        .await;
    assert_ne!(ran["ok"], true, "a dismissed record ran: {ran}");
    let twice = client
        .manage(json!({ "op": "native.resume.dismiss", "node": NODE, "id": record["id"] }))
        .await;
    assert_ne!(twice["ok"], true, "{twice}");
}
