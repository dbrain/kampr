use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Set on `grid.*` frames only — the ones a purge is allowed to throw away, because a
    /// `grid.reset` makes every dropped patch for that pane irrelevant, and the only ones a
    /// stopped pane may be refused: dropping a `styles` frame would leave the connection's pen
    /// table behind the encoder's for good.
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
///
/// **The bulk lane is a second queue that every ordinary frame overtakes.** Attachment chunks go
/// there and nothing else does: a 2.22 MB record (#247) queued in front of the terminal frames
/// would stop every pane on the connection repainting for as long as the link takes to drain it,
/// which is the whole reason the local attachment route is HTTP rather than a wire message. It is
/// drained only when nothing else is waiting, so a transfer costs a frame the time to write one
/// chunk rather than the time to write the record.
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
    bulk: VecDeque<Frame>,
    stopped: HashSet<String>,
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
        if frame
            .pane
            .as_deref()
            .is_some_and(|pane| inner.stopped.contains(pane))
        {
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

    /// A frame nothing on this connection has to wait behind.
    ///
    /// Only an attachment's chunks go here. The producer is credit-driven, so a `false` is the
    /// end of that transfer and not of the connection: a bulk lane that would not drain is a
    /// transfer that stalls and times out at the hub, never a pane that stops.
    pub fn push_bulk(&self, frame: Frame) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed || inner.bulk.len() >= self.cap {
            return false;
        }
        inner.bulk.push_back(frame);
        drop(inner);
        self.ready.notify_one();
        true
    }

    pub async fn next(&self) -> Option<Frame> {
        loop {
            let waiting = self.ready.notified();
            {
                let mut inner = self.inner.lock().unwrap();
                let next = match inner.queue.pop_front() {
                    Some(frame) => Some(frame),
                    None => inner.bulk.pop_front(),
                };
                if let Some(frame) = next {
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
        let dropped = Self::drop_pane(&mut inner, pane);
        if dropped > 0 {
            inner.purges += 1;
            inner.dropped += dropped as u64;
        }
        dropped
    }

    /// Unwatching a pane aborts its pump, and `JoinHandle::abort` is not synchronous: an iteration
    /// already running on another worker runs to its next await, and there is no await between
    /// taking an update off the watcher and enqueueing it. Emptying the queue cannot catch that
    /// frame, because it is not in the queue yet — so the stop is recorded here instead, the one
    /// place where a push and a stop are serialised against each other.
    pub fn stop_pane(&self, pane: &str) {
        let mut inner = self.inner.lock().unwrap();
        Self::drop_pane(&mut inner, pane);
        inner.stopped.insert(pane.to_string());
    }

    pub fn resume_pane(&self, pane: &str) {
        self.inner.lock().unwrap().stopped.remove(pane);
    }

    fn drop_pane(inner: &mut Inner, pane: &str) -> usize {
        let before = inner.queue.len();
        inner.queue.retain(|f| f.pane.as_deref() != Some(pane));
        before - inner.queue.len()
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

    /// The property the whole relayed-attachment path rests on: a chunk already queued never
    /// delays a frame that is enqueued after it.
    #[tokio::test]
    async fn a_frame_overtakes_every_attachment_chunk_already_queued() {
        let o = Outbox::new(16);
        for n in 0..8 {
            assert!(o.push_bulk(Frame::plain(format!(r#"{{"t":"att.chunk","seq":{n}}}"#))));
        }
        o.push(patch("p1", 1));
        assert_eq!(o.next().await.unwrap(), patch("p1", 1));
        assert_eq!(o.next().await.unwrap().json, r#"{"t":"att.chunk","seq":0}"#);
    }

    /// Bulk frames are not the client falling behind on its panes, so they must not trip the rule
    /// that purges a pane and re-sends its grid.
    #[test]
    fn a_transfer_in_progress_is_not_congestion() {
        let o = Outbox::new(4);
        for n in 0..4 {
            assert!(o.push_bulk(Frame::plain(format!(r#"{{"seq":{n}}}"#))));
        }
        assert!(!o.congested());
        assert!(
            !o.push_bulk(Frame::plain(r#"{"seq":4}"#.into())),
            "the bulk lane grew past its own bound",
        );
        assert!(!o.is_closed(), "a full bulk lane ended the connection");
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
