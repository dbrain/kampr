package dev.kampr.shared.net

// Two clocks, because neither one answers both questions. `monotonic` cannot see a background: on
// Android it is `System.nanoTime()`, which is CLOCK_MONOTONIC and stops while the device is in deep
// sleep, so a phone that was away for five minutes comes back believing no time passed. `wall` does
// count that sleep, and is the only clock that can measure it — but it steps when NTP corrects it,
// so it is never used to decide that a socket that is answering is dead.
//
// The four windows, and where each number comes from:
//
// `silenceDeadlineMs` is counted against a `pingIntervalMs` heartbeat, so 25s is "two pings went
// unanswered, and the second had five seconds of slack on top". One lost ping is a lost packet or
// a radio waking up. Under 20s a single stall on a phone rejoining a network would close a session
// that was about to answer; much over 30s and the operator is back to watching a stale pane, which
// is the whole complaint.
//
// `resumeQuietMs` has to clear the widest silence a *healthy* socket can show, which is one ping
// interval plus a round trip: below 12s the connection has not even missed a heartbeat, and asking
// it anything is a round trip bought for nothing.
//
// `probeMs` is the one that decides it. The app is in front and the screen is on, so 3s is a very
// slow round trip rather than a dead one, and it is what bounds the whole recovery: 3s to decide,
// then the ladder's first rung.
class Liveness(
    val pingIntervalMs: Double = 10_000.0,
    val silenceDeadlineMs: Double = 25_000.0,
    val resumeQuietMs: Double = 12_000.0,
    val probeMs: Double = 3_000.0,
    val tickMs: Long = 1_000,
    val monotonic: () -> Double = ::nowMillis,
    val wall: () -> Double = ::wallClockMillis,
)
