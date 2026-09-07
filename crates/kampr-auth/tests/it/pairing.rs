//! Where the pairing code's key derivation actually runs.

use kampr_auth::{AuditLog, Auth, Policy, Store, Tier};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    let turns = Arc::new(AtomicU64::new(0));

    let watcher = tokio::spawn({
        let done = done.clone();
        let turns = turns.clone();
        async move {
            let mut longest = Duration::ZERO;
            while !done.load(Ordering::Relaxed) {
                let at = Instant::now();
                tokio::task::yield_now().await;
                longest = longest.max(at.elapsed());
                turns.fetch_add(1, Ordering::Relaxed);
            }
            longest
        }
    });
    tokio::task::yield_now().await;

    // **Counted across the redemption and nothing else.** What this is about is whether another
    // task can run *while the derivation does*, so the window has to be the derivation — a count
    // over the whole test is dominated by the spinning either side of it and a fifteen-millisecond
    // block barely dents it, which is a guard that passes with the defect in place.
    let before = turns.load(Ordering::Relaxed);
    let _ = a.redeem_pairing("ZZZZ-ZZZZ", "attacker", None, "9.9.9.9").await;
    let during = turns.load(Ordering::Relaxed) - before;
    done.store(true, Ordering::Relaxed);

    let longest = watcher.await.unwrap();
    // **Turns, not milliseconds.** The old bound was 5 ms against a healthy longest gap of 70 µs
    // and a whole redemption of ~15 ms, so it sat three times above an ordinary scheduler hiccup
    // and three times below the defect. CI stalled for **5.027 ms** on a commit that changed one
    // Kotlin label, and a release stopped for it. Thousands of turns pass during a healthy
    // redemption and none pass during one that holds this thread, so the floor has two orders of
    // magnitude either side of it and no opinion about how fast the runner is.
    assert!(
        during > 100,
        "the derivation ran on the only thread there was: the rest of the runtime got {during} \
         turns while it worked, and its longest wait was {longest:?}"
    );
}
