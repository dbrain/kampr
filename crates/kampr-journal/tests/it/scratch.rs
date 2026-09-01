//! The test harness's own hygiene.
//!
//! Every scratch directory here lives on a tmpfs shared with the whole machine, and a suite that
//! strands them is a suite that quietly fills it: a full run used to leave thousands behind,
//! gigabytes of them, until an unrelated crate started failing with `Disk quota exceeded`.

use crate::common;
use crate::common::scratch_dir;

#[test]
fn a_scratch_directory_goes_away_with_the_value_that_owns_it() {
    let path = {
        let dir = scratch_dir("guard");
        std::fs::write(dir.join("something"), "bytes").expect("a file in it");
        dir.to_path_buf()
    };

    assert!(!path.exists(), "{} outlived its guard", path.display());
}

/// The case that decides whether this holds in the suite that needs it. A test that fails leaves
/// the most behind, and unwinding runs destructors — so the directory has to go with the panic
/// rather than only with a pass.
#[test]
fn a_scratch_directory_goes_away_when_its_test_panics() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(std::path::PathBuf::new()));
    let recorded = seen.clone();
    let outcome = std::panic::catch_unwind(move || {
        let dir = scratch_dir("guard-panic");
        std::fs::write(dir.join("something"), "bytes").expect("a file in it");
        *recorded.lock().unwrap() = dir.to_path_buf();
        panic!("the test this stands in for");
    });

    assert!(outcome.is_err(), "this has to be the panicking path");
    let path = seen.lock().unwrap().clone();
    assert!(
        path.as_os_str().is_empty() || !path.exists(),
        "{} survived a panic",
        path.display()
    );
}

/// A transcript fixture's root has a parent of its own, so the tests that write *outside* the
/// root — an escape target, a symlink's destination — write inside the guard rather than into the
/// shared temp directory under a name every parallel test would share.
#[test]
fn everything_a_fixture_writes_beside_its_root_is_inside_the_guard() {
    let temp = std::env::temp_dir();
    let scratch = common::scratch_claude("guard-parent", &[]);
    let beside = scratch.root.parent().expect("a parent").to_path_buf();

    assert_ne!(
        beside, temp,
        "a fixture's root sits directly in the temp directory, so anything written beside it is \
         stranded there and collides with every other test doing the same"
    );
    assert!(beside.starts_with(&temp), "{}", beside.display());
}
