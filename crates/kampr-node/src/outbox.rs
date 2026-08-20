use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Set on `grid.*` frames only — the ones a purge is allowed to throw away, because a
    /// `grid.reset` makes every dropped patch for that pane irrelevant.
    pub pane: Option<String>,
    pub json: String,
}

impl Frame {
    pub fn plain(json: String) -> Self {
        Self { pane: None, json }
    }

    pub fn grid(pane: &str, json: String) -> Self {
        Self {
            pane: Some(pane.to_string()),
            json,
        }
    }
}

/// A per-connection send queue with a ceiling.
///
/// A phone on a slow link must not be able to grow the node's memory, so the queue is bounded and
/// the overflow policy is to *drop the pane's patch queue and send one `grid.reset`* rather than
/// to buffer. That is cheap by construction: a full grid is a few kilobytes and Herdr coalesces
/// bursts to end state, so a reset is never more expensive than the patches it replaces.
///
/// The hard cap is the floor beneath that. Frames that are not `grid.*` — pongs, errors, herd
/// patches — cannot be dropped without lying to the client, so a connection that will not drain
/// them is closed instead.
#[derive(Debug)]
pub struct Outbox {
    inner: Mutex<Inner>,
    ready: Notify,
    cap: usize,
    hard_cap: usize,
}

#[derive(Debug, Default)]
struct Inner {
    queue: VecDeque<Frame>,
    closed: bool,
    purges: u64,
    dropped: u64,
}

impl Outbox {
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(2);
        Self {
            inner: Mutex::new(Inner::default()),
            ready: Notify::new(),
            cap,
            hard_cap: cap.saturating_mul(4),
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    /// False once the connection is finished, which is the signal for a producer to stop.
    pub fn push(&self, frame: Frame) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return false;
        }
        if inner.queue.len() >= self.hard_cap {
            inner.closed = true;
            drop(inner);
            self.ready.notify_waiters();
            return false;
        }
        inner.queue.push_back(frame);
        drop(inner);
        self.ready.notify_one();
        true
    }

    pub async fn next(&self) -> Option<Frame> {
        loop {
            let waiting = self.ready.notified();
            {
                let mut inner = self.inner.lock().unwrap();
                if let Some(frame) = inner.queue.pop_front() {
                    return Some(frame);
                }
                if inner.closed {
                    return None;
                }
            }
            waiting.await;
        }
    }

    pub fn close(&self) {
        self.inner.lock().unwrap().closed = true;
        self.ready.notify_waiters();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    pub fn depth(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }

    /// The client is not keeping up. Producers check this *before* encoding, so an overflowing
    /// connection costs a purge and a reset rather than a queue of patches it can never drain.
    pub fn congested(&self) -> bool {
        self.inner.lock().unwrap().queue.len() >= self.cap
    }

    pub fn purge_pane(&self, pane: &str) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.queue.len();
        inner.queue.retain(|f| f.pane.as_deref() != Some(pane));
        let dropped = before - inner.queue.len();
        if dropped > 0 {
            inner.purges += 1;
            inner.dropped += dropped as u64;
        }
        dropped
    }

    pub fn stats(&self) -> (u64, u64) {
        let inner = self.inner.lock().unwrap();
        (inner.purges, inner.dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(pane: &str, n: usize) -> Frame {
        Frame::grid(pane, format!(r#"{{"t":"grid.patch","pane":"{pane}","n":{n}}}"#))
    }

    #[tokio::test]
    async fn frames_come_out_in_order() {
        let o = Outbox::new(8);
        o.push(patch("p1", 1));
        o.push(patch("p1", 2));
        assert_eq!(o.next().await.unwrap(), patch("p1", 1));
        assert_eq!(o.next().await.unwrap(), patch("p1", 2));
    }

    #[tokio::test]
    async fn next_parks_until_a_frame_arrives() {
        let o = std::sync::Arc::new(Outbox::new(8));
        let pusher = {
            let o = o.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                o.push(patch("p1", 1));
            })
        };
        assert_eq!(o.next().await.unwrap(), patch("p1", 1));
        pusher.await.unwrap();
    }

    #[test]
    fn congestion_is_reported_at_the_soft_cap() {
        let o = Outbox::new(4);
        for n in 0..3 {
            o.push(patch("p1", n));
        }
        assert!(!o.congested());
        o.push(patch("p1", 3));
        assert!(o.congested());
    }

    #[test]
    fn purging_a_pane_leaves_every_other_frame_alone() {
        let o = Outbox::new(16);
        o.push(Frame::plain(r#"{"t":"pong","n":1}"#.into()));
        o.push(patch("p1", 1));
        o.push(patch("p2", 1));
        o.push(patch("p1", 2));
        assert_eq!(o.purge_pane("p1"), 2);
        assert_eq!(o.depth(), 2);
        assert_eq!(o.stats(), (1, 2));
    }

    #[test]
    fn a_client_that_never_drains_is_closed_rather_than_buffered() {
        let o = Outbox::new(4);
        // Undroppable frames: nothing but closing the connection bounds these.
        for n in 0..64 {
            if !o.push(Frame::plain(format!(r#"{{"t":"pong","n":{n}}}"#))) {
                assert!(o.is_closed());
                assert!(
                    o.depth() <= o.cap() * 4,
                    "the queue stopped growing at the hard cap"
                );
                return;
            }
        }
        panic!("the outbox buffered without limit");
    }

    #[tokio::test]
    async fn a_closed_outbox_drains_what_it_has_and_then_ends() {
        let o = Outbox::new(8);
        o.push(patch("p1", 1));
        o.close();
        assert!(o.next().await.is_some());
        assert!(o.next().await.is_none());
        assert!(!o.push(patch("p1", 2)));
    }
}
