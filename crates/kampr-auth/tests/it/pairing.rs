//! Where the pairing code's key derivation actually runs.

use kampr_auth::{AuditLog, Auth, Policy, Store, Tier};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// argon2id at 19 MiB is tens of milliseconds of one core, and `/auth/pair` hands it a code
/// chosen by somebody who has proved nothing. Run inline it is that long with the calling thread
/// held — which on a node is a tokio worker, and a few hundred of them is every worker.
///
/// A current-thread runtime is the honest instrument: there is exactly one, so anything the
/// redemption does inline is time no other task can have.
#[tokio::test]
async fn redeeming_a_pairing_code_leaves_the_thread_that_asked_free() {
    let a = Arc::new(
        Auth::new(
            Store::open_memory().await.unwrap(),
            Tier::detect("http://192.168.1.24:8790").unwrap(),
            AuditLog::disabled(),
            Policy::default(),
            &[],
        )
        .unwrap(),
    );
    let done = Arc::new(AtomicBool::new(false));

    let watcher = tokio::spawn({
        let done = done.clone();
        async move {
            let mut longest = Duration::ZERO;
            while !done.load(Ordering::Relaxed) {
                let at = Instant::now();
                tokio::task::yield_now().await;
                longest = longest.max(at.elapsed());
            }
            longest
        }
    });
    tokio::task::yield_now().await;

    let _ = a.redeem_pairing("ZZZZ-ZZZZ", "attacker", None, "9.9.9.9").await;
    done.store(true, Ordering::Relaxed);

    let longest = watcher.await.unwrap();
    assert!(
        longest < Duration::from_millis(5),
        "the derivation ran on the only thread there was: nothing else got a turn for {longest:?}"
    );
}
