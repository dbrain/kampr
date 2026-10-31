//! Workspaces, tabs, agents, worktrees and layouts on Kampr sessions: herdr's structure ops, served
//! by the node itself so herdr is the optional extra rather than the only way to have any.

use super::*;

fn local(id: &Value) -> String {
    id.as_str()
        .and_then(|id| id.strip_prefix(&format!("{NODE}/")))
        .unwrap_or_else(|| panic!("not one of this node's ids: {id}"))
        .to_string()
}

impl Client {
    async fn made(&mut self, op: Value) -> (Value, Value) {
        let ack = self.manage(op.clone()).await;
        assert_eq!(ack["ok"], json!(true), "{op}: {ack}");
        let id = ack["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{op}: no id in {ack}"))
            .to_string();
        let pane = self.pane(&id, |_| true).await;
        (ack, pane)
    }
}

/// The operator: *"let's go full kampr and herdr is the help you migrate piece"*. A workspace and
/// its tabs are made of Kampr sessions, read back on the panes the way herdr's are, and survive the
/// node restarting.
#[tokio::test]
async fn a_workspace_holds_tabs_of_kampr_sessions_and_keeps_them_across_a_restart() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let (_, first) = client
        .made(json!({ "op": "workspace.create", "label": "api", "cwd": h.work() }))
        .await;
    assert_eq!(first["provider"], "native", "{first}");
    assert_eq!(first["workspace"], "api", "{first}");
    let workspace = first["workspace_id"].clone();
    assert!(local(&workspace).starts_with("kw:"), "{first}");
    assert_eq!(first["tab"], "1", "{first}");

    let (_, second) = client.made(json!({ "op": "tab.create", "at": workspace })).await;
    assert_eq!(second["workspace_id"], workspace, "{second}");
    assert_eq!(second["tab"], "2", "{second}");
    assert_ne!(second["tab_id"], first["tab_id"], "{second}");

    let split = client
        .manage(json!({ "op": "pane.split", "at": second["id"], "direction": "right" }))
        .await;
    assert_eq!(split["ok"], json!(true), "{split}");
    let beside = client
        .pane(split["id"].as_str().expect("an id"), |p| {
            p["tab_id"] == second["tab_id"]
        })
        .await;
    assert_eq!(beside["workspace_id"], workspace, "{beside}");

    let (_, joined) = client
        .made(json!({ "op": "native.create", "node": NODE, "args": ["sh"], "workspace": "api" }))
        .await;
    assert_eq!(
        joined["workspace_id"], workspace,
        "a name joins the workspace: {joined}"
    );

    let renamed = client
        .manage(json!({ "op": "rename", "at": workspace, "label": "backend" }))
        .await;
    assert_eq!(renamed["ok"], json!(true), "{renamed}");
    client
        .pane(first["id"].as_str().unwrap(), |p| p["workspace"] == "backend")
        .await;

    drop(client);
    let h = h.restart().await;
    let mut client = h.client().await;
    for (pane, tab) in [(&first, "1"), (&second, "2")] {
        let back = client
            .pane(pane["id"].as_str().unwrap(), |p| p["workspace"] == "backend")
            .await;
        assert_eq!(back["workspace_id"], workspace, "{back}");
        assert_eq!(back["tab"], tab, "{back}");
    }

    let closed = client.manage(json!({ "op": "close", "at": workspace })).await;
    assert_eq!(closed["ok"], json!(true), "{closed}");
    for pane in [&first, &second, &joined, &beside] {
        client.gone(pane["id"].as_str().unwrap()).await;
    }
}

/// An agent is a Kampr session running its harness: beside the pane it was started from, the way
/// herdr starts one, and offered only where its binary is on the node's PATH.
#[tokio::test]
async fn an_agent_starts_as_a_kampr_session_beside_the_pane_and_only_where_it_is_installed() {
    let h = Harness::start_with(Harness::home(), |c| {
        let bin = PathBuf::from(&c.native.worktrees).with_file_name("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, "#!/bin/sh\necho \"fake claude $*\"\nexec cat\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        c.fleet.path = format!("{}:/usr/bin:/bin", bin.display());
    })
    .await;
    let mut client = h.client().await;
    client.send(json!({ "t": "caps" })).await;
    let caps = client.reply("caps").await;
    assert_eq!(caps["native_agents"], json!(["claude"]), "{caps}");
    assert!(
        caps["agent_kinds"]
            .as_array()
            .is_some_and(|k| k.contains(&json!("claude"))),
        "{caps}"
    );

    let (_, shell) = client
        .made(json!({ "op": "workspace.create", "label": "work" }))
        .await;
    let (_, agent) = client
        .made(json!({ "op": "agent.start", "kind": "claude", "at": shell["id"], "args": ["--resume"] }))
        .await;
    assert_eq!(agent["provider"], "native", "{agent}");
    assert_eq!(agent["workspace_id"], shell["workspace_id"], "{agent}");
    let id = agent["id"].as_str().unwrap().to_string();
    client.watch(&id, false).await;
    client.shows(&id, "fake claude --resume").await;
    client.pane(&id, |p| p["tab_id"] == shell["tab_id"]).await;

    let missing = client
        .manage(json!({ "op": "agent.start", "kind": "codex" }))
        .await;
    assert_eq!(missing["ok"], json!(false), "{missing}");
    assert!(
        missing["message"].as_str().unwrap_or_default().contains("codex"),
        "{missing}"
    );
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A worktree is a branch checked out beside the repo and a workspace named for it, laid out as
/// herdr lays its own out (#617): `<repo>/<branch>` with a slash in the branch made a dash.
#[tokio::test]
async fn a_worktree_is_a_checkout_and_a_workspace_named_for_its_branch() {
    let h = Harness::start().await;
    let repo = h.work().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    let mut client = h.client().await;

    let (ack, pane) = client
        .made(json!({ "op": "worktree.create", "cwd": repo, "branch": "feat/y" }))
        .await;
    let path = PathBuf::from(&h.node.config.native.worktrees)
        .join("repo")
        .join("feat-y");
    assert_eq!(pane["workspace"], "feat-y", "{pane}");
    assert_eq!(pane["cwd"], json!(path), "{pane}");
    assert!(git(&repo, &["worktree", "list"]).contains(&path.display().to_string()));
    assert!(git(&path, &["branch", "--show-current"]).trim() == "feat/y");
    let _ = ack;

    let (_, again) = client.made(json!({ "op": "worktree.open", "path": path })).await;
    assert_eq!(again["workspace"], "feat-y", "{again}");
    assert_eq!(again["cwd"], json!(path), "{again}");
}

/// A tab's split tree goes out in the shape herdr's `layout.export` has (#47) and comes back as a
/// new tab of fresh sessions split the same way.
#[tokio::test]
async fn a_tabs_layout_exports_and_applies_as_a_new_tab_split_the_same_way() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let (_, first) = client
        .made(json!({ "op": "workspace.create", "label": "lay" }))
        .await;
    let split = client
        .manage(json!({ "op": "pane.split", "at": first["id"], "direction": "down", "ratio": 0.3 }))
        .await;
    assert_eq!(split["ok"], json!(true), "{split}");
    let second = client
        .pane(split["id"].as_str().unwrap(), |p| p["tab_id"] == first["tab_id"])
        .await;
    let first = client
        .pane(first["id"].as_str().unwrap(), |p| p["tile"].is_object())
        .await;

    let exported = client
        .manage(json!({ "op": "layout.export", "at": first["tab_id"] }))
        .await;
    assert_eq!(exported["ok"], json!(true), "{exported}");
    let root = &exported["layout"]["root"];
    assert_eq!(
        (
            &root["type"],
            &root["direction"],
            &root["first"]["type"],
            &root["second"]["type"]
        ),
        (&json!("split"), &json!("down"), &json!("pane"), &json!("pane")),
        "{exported}"
    );

    let applied = client
        .manage(json!({ "op": "layout.apply", "at": first["workspace_id"], "layout": exported["layout"] }))
        .await;
    assert_eq!(applied["ok"], json!(true), "{applied}");
    let made = client
        .pane(applied["id"].as_str().expect("an id"), |p| p["tile"].is_object())
        .await;
    assert_eq!(made["workspace_id"], first["workspace_id"], "{made}");
    assert_ne!(made["tab_id"], first["tab_id"], "{made}");
    assert_eq!(made["tab"], "2", "{made}");
    assert_eq!(
        made["tile"]["path"], first["tile"]["path"],
        "{made} against {second}"
    );
}

/// Kampr first: an op that names nothing of herdr's is Kampr's even where herdr is on, and the
/// client's herdr section says `provider: "herdr"` to reach herdr — refused for being off where it
/// is off, and tried against herdr where it is on.
#[tokio::test]
async fn a_structure_op_is_kamprs_unless_it_names_herdr_or_something_of_herdrs() {
    let off = Harness::start().await;
    let mut client = off.client().await;
    let refused = client
        .manage(json!({ "op": "workspace.create", "label": "w", "provider": "herdr" }))
        .await;
    assert_eq!(refused["code"], "herdr_unavailable", "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("herdr is off"),
        "{refused}"
    );
    drop(client);
    drop(off);

    let on = Harness::start_with(Harness::home(), |c| c.herdr.enabled = Some(true)).await;
    let mut client = on.client().await;
    let (_, pane) = client
        .made(json!({ "op": "workspace.create", "label": "mine" }))
        .await;
    assert_eq!(pane["provider"], "native", "{pane}");
    let tried = client
        .manage(json!({ "op": "workspace.create", "label": "theirs", "provider": "herdr" }))
        .await;
    assert_eq!(tried["code"], "herdr_unavailable", "{tried}");
    assert!(
        !tried["message"].as_str().unwrap().contains("herdr is off"),
        "a herdr that is on was reported off: {tried}"
    );
}
