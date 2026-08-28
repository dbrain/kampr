use crate::peers::RemoteEvent;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Notify;

/// One relayed pane's stream, handed to every watcher of it.
///
/// **A grid frame is droppable and nothing else is.** `tokio::sync::broadcast` drops the *oldest*
/// events for a receiver that falls behind, whatever they are, and this hop used to be one: a
/// client that overran it was caught up with a fresh grid out of the hub's shadow, and the
/// `convo`, `convo.turn`, `pending` and `scrollback` messages that went down with the grid frames
/// had nowhere to come back from — the node that owns the pane recorded them as delivered the
/// moment it handed them to the hub. So this is `kampr_node::outbox`'s rule, one hop earlier: an
/// overrun drops that watcher's queued grid frames and leaves a marker where they were, and the
/// marker becomes one full grid out of the shadow when it is read.
///
/// Raising the depth would only widen the window. The floor beneath the rule is therefore the
/// same one the outbox has: a watcher still over the ceiling once its grid frames are gone is one
/// that is not reading at all, and it is ended rather than buffered without limit.
#[derive(Debug)]
pub struct Fanout {
    cap: usize,
    subscribers: Mutex<Vec<Weak<Subscriber>>>,
}

/// Undroppable events a watcher may be behind by before it is ended, as a multiple of the depth.
const CEILING: usize = 4;

#[derive(Debug)]
pub enum Delivery {
    Event(RemoteEvent),
    /// Grid frames were dropped for this watcher; it rejoins at one full grid from the shadow.
    Resync(usize),
    Ended,
}

impl Fanout {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(2),
            subscribers: Mutex::new(Vec::new()),
        }
    }

    pub fn subscribe(&self) -> Arc<Subscriber> {
        let subscriber = Arc::new(Subscriber::default());
        self.subscribers.lock().unwrap().push(Arc::downgrade(&subscriber));
        subscriber
    }

    pub fn send(&self, event: RemoteEvent) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|weak| match weak.upgrade() {
                Some(subscriber) => {
                    subscriber.push(event.clone(), self.cap);
                    true
                }
                None => false,
            });
    }
}

#[derive(Debug, Default)]
pub struct Subscriber {
    queue: Mutex<Queue>,
    ready: Notify,
}

#[derive(Debug, Default)]
struct Queue {
    events: VecDeque<Queued>,
    ended: bool,
    overrun: bool,
}

#[derive(Debug)]
enum Queued {
    Event(RemoteEvent),
    Resync(usize),
}

impl Subscriber {
    pub async fn recv(&self) -> Delivery {
        loop {
            // Registered before the queue is looked at, so a push landing in between is a wakeup
            // this waiter still gets.
            let waiting = self.ready.notified();
            if let Some(delivery) = self.take() {
                return delivery;
            }
            waiting.await;
        }
    }

    pub fn overrun(&self) -> bool {
        self.queue.lock().unwrap().overrun
    }

    fn take(&self) -> Option<Delivery> {
        let mut queue = self.queue.lock().unwrap();
        match queue.events.pop_front() {
            Some(Queued::Event(event)) => Some(Delivery::Event(event)),
            Some(Queued::Resync(dropped)) => Some(Delivery::Resync(dropped)),
            None => queue.ended.then_some(Delivery::Ended),
        }
    }

    fn push(&self, event: RemoteEvent, cap: usize) {
        let mut queue = self.queue.lock().unwrap();
        if queue.ended {
            return;
        }
        queue.events.push_back(Queued::Event(event));
        if queue.events.len() > cap {
            queue.coalesce();
        }
        // What is left is undroppable by rule, so a watcher still over the ceiling is one that is
        // not reading. It keeps what it is owed and is told the stream ended rather than being
        // handed a transcript with a hole in it.
        if queue.events.len() > cap * CEILING {
            queue.ended = true;
            queue.overrun = true;
        }
        drop(queue);
        self.ready.notify_one();
    }
}

impl Queue {
    fn coalesce(&mut self) {
        let before = self.events.len();
        self.events
            .retain(|queued| !matches!(queued, Queued::Event(RemoteEvent::Update(_))));
        let dropped = before - self.events.len();
        if dropped == 0 {
            return;
        }
        match self.events.iter_mut().find_map(|q| match q {
            Queued::Resync(already) => Some(already),
            Queued::Event(_) => None,
        }) {
            Some(already) => *already += dropped,
            // Behind whatever survived, so the grid a watcher rejoins at is never applied before
            // an event that was already ahead of it.
            None => self.events.push_back(Queued::Resync(dropped)),
        }
    }
}
