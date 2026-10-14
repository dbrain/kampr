use crate::repo;

/// The modules `.config/nextest.toml` leaves out of the default profile. A test that needs a real
/// herdr anywhere else would make `make test-fast` fail on a machine without one.
const LIVE: &[&str] = &[
    "crates/kampr-cli/tests/it/doctor_live.rs",
    "crates/kampr-core/tests/it/events.rs",
    "crates/kampr-node/tests/it/live.rs",
    "crates/kampr-node/tests/it/mesh.rs",
];

fn needs_herdr(text: &str) -> bool {
    text.contains("herdr_on_path()") || text.contains("Command::new(\"herdr\")")
}

#[test]
fn every_test_that_needs_herdr_is_in_a_module_the_fast_profile_skips() {
    let root = repo::root();
    let found: Vec<String> = repo::files(&root.join("crates"), &["rs"])
        .into_iter()
        .filter(|path| path.components().any(|c| c.as_os_str() == "tests"))
        .filter(|path| !path.starts_with(root.join("crates/kampr-testkit")))
        .filter(|path| needs_herdr(&repo::read(path)))
        .map(|path| repo::relative(&path))
        .collect();
    assert_eq!(
        found, LIVE,
        "update LIVE and the filter in .config/nextest.toml together"
    );
    let config = repo::read(&root.join(".config/nextest.toml"));
    for module in ["doctor_live", "events", "live|mesh"] {
        assert!(config.contains(module), "nextest.toml no longer names {module}");
    }
}
