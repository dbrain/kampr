use std::collections::{BTreeMap, BTreeSet};

/// Each crate and the workspace crates it may depend on at runtime. Dev-dependencies are free:
/// a test may stand up a whole node. A crate that needs a new edge should first ask whether the
/// thing it wants belongs lower down.
const ALLOWED: &[(&str, &[&str])] = &[
    ("kampr-auth", &[]),
    ("kampr-herdr", &[]),
    ("kampr-journal", &[]),
    ("kampr-keep", &[]),
    ("kampr-term", &[]),
    ("kampr-testkit", &[]),
    ("kampr-push", &["kampr-auth"]),
    ("kampr-core", &["kampr-herdr", "kampr-journal", "kampr-term"]),
    ("kampr-fleet", &["kampr-core", "kampr-keep", "kampr-native"]),
    ("kampr-mesh", &["kampr-auth", "kampr-core", "kampr-term"]),
    (
        "kampr-native",
        &["kampr-core", "kampr-journal", "kampr-keep", "kampr-term"],
    ),
    ("kampr-spike", &["kampr-herdr", "kampr-term"]),
    (
        "kampr-node",
        &[
            "kampr-auth",
            "kampr-core",
            "kampr-fleet",
            "kampr-herdr",
            "kampr-journal",
            "kampr-keep",
            "kampr-mesh",
            "kampr-native",
            "kampr-push",
            "kampr-term",
        ],
    ),
    (
        "kampr-client",
        &[
            "kampr-auth",
            "kampr-core",
            "kampr-journal",
            "kampr-mesh",
            "kampr-term",
        ],
    ),
    ("kampr-tui", &["kampr-client", "kampr-core", "kampr-term"]),
    (
        "kampr-cli",
        &[
            "kampr-auth",
            "kampr-client",
            "kampr-core",
            "kampr-fleet",
            "kampr-herdr",
            "kampr-keep",
            "kampr-mesh",
            "kampr-native",
            "kampr-node",
            "kampr-push",
            "kampr-tui",
        ],
    ),
];

type Graph = BTreeMap<String, BTreeSet<String>>;

fn workspace() -> Graph {
    let out = std::process::Command::new(env!("CARGO"))
        .current_dir(crate::repo::root())
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline"])
        .output()
        .expect("cargo metadata");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).expect("metadata json");
    let packages = meta["packages"].as_array().expect("packages");
    let members: BTreeSet<&str> = packages.iter().filter_map(|p| p["name"].as_str()).collect();
    packages
        .iter()
        .map(|p| {
            let deps = p["dependencies"]
                .as_array()
                .expect("dependencies")
                .iter()
                .filter(|d| d["kind"].is_null())
                .filter_map(|d| d["name"].as_str())
                .filter(|name| members.contains(name))
                .map(str::to_string)
                .collect();
            (p["name"].as_str().expect("name").to_string(), deps)
        })
        .collect()
}

fn allowed() -> Graph {
    ALLOWED
        .iter()
        .map(|(name, deps)| (name.to_string(), deps.iter().map(|d| d.to_string()).collect()))
        .collect()
}

fn violations(actual: &Graph, allowed: &Graph) -> Vec<String> {
    let mut out = Vec::new();
    for (name, deps) in actual {
        let Some(permitted) = allowed.get(name) else {
            out.push(format!("{name} is not in ALLOWED"));
            continue;
        };
        out.extend(
            deps.difference(permitted)
                .map(|d| format!("{name} -> {d} is not allowed")),
        );
        out.extend(
            permitted
                .difference(deps)
                .map(|d| format!("{name} -> {d} is allowed but unused")),
        );
    }
    out
}

#[test]
fn the_crate_graph_is_the_one_written_down() {
    let faults = violations(&workspace(), &allowed());
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

#[test]
fn a_new_edge_and_a_dropped_one_are_both_reported() {
    let mut actual = allowed();
    actual
        .get_mut("kampr-client")
        .unwrap()
        .insert("kampr-node".into());
    actual.get_mut("kampr-tui").unwrap().remove("kampr-term");
    assert_eq!(
        violations(&actual, &allowed()),
        vec![
            "kampr-client -> kampr-node is not allowed",
            "kampr-tui -> kampr-term is allowed but unused",
        ]
    );
}
