use crate::note::{Blocked, Notification};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::mpsc;

/// How long a first change waits for company.
///
/// Long enough that three agents finishing a batch of edits arrive as one notification, short
/// enough that a lone blocked agent is not held back noticeably. It is a *collection* window, not
/// a rate limit: the window starts at the first change and does not extend, so a steady trickle
/// still gets a notification every window rather than never.
pub const WINDOW: Duration = Duration::from_millis(900);

/// What the blocked set did.
///
/// **The payload is the set, not the edge.** A notification replaces its predecessor under one
/// tag, so a notification naming only what just changed silently unsays everything it does not
/// name — a second agent blocking used to take the first one off the phone, and an agent answered
/// at the desk stayed on it until somebody tapped. `outstanding` is what the device should be
/// showing; `fresh` and `cleared` only decide whether it buzzes and who has to be told.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Every pane still blocked, in the herd's own order.
    pub outstanding: Vec<Blocked>,
    /// Which of `outstanding` were not blocked last time. Non-empty is what makes a phone buzz.
    pub fresh: HashSet<String>,
    /// Panes that stopped being blocked. They are gone from the herd, so their ids are all there
    /// is — and their ids are enough, because what they are for is finding the devices that were
    /// told about them.
    pub cleared: HashSet<String>,
}

impl Change {
    /// Everything outstanding is news: the shape a first block takes, and the one a test writes.
    pub fn fresh(outstanding: Vec<Blocked>) -> Self {
        Self {
            fresh: outstanding.iter().map(|p| p.pane.clone()).collect(),
            outstanding,
            cleared: HashSet::new(),
        }
    }

    pub fn cleared(outstanding: Vec<Blocked>, cleared: impl IntoIterator<Item = String>) -> Self {
        Self {
            outstanding,
            fresh: HashSet::new(),
            cleared: cleared.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fresh.is_empty() && self.cleared.is_empty()
    }

    /// Folds a later change into this one. The later `outstanding` is simply the truth; what has
    /// to be carried is *who has to hear about it*, which the later change alone does not know.
    ///
    /// A pane that blocked and unblocked inside one window is dropped from both sets: nothing was
    /// ever shown for it, so there is nothing to alert about and nothing to take down.
    fn absorb(&mut self, next: Change) {
        let ids: HashSet<String> = next.outstanding.iter().map(|p| p.pane.clone()).collect();
        self.fresh.extend(next.fresh);
        self.cleared.extend(next.cleared);
        let transient: HashSet<String> = self.fresh.intersection(&self.cleared).cloned().collect();
        self.fresh
            .retain(|id| ids.contains(id) && !transient.contains(id));
        self.cleared
            .retain(|id| !ids.contains(id) && !transient.contains(id));
        self.outstanding = next.outstanding;
    }
}

/// One change, split by who may receive which pane.
///
/// A device that muted one of three blocked agents must see a notification naming the other two,
/// not the whole set and not nothing. So the split is per subscription and the notification is
/// built from that subscription's own eligible panes.
///
/// `eligible` must carry the cleared panes as well as the outstanding ones. A device eligible for
/// a pane that just cleared is exactly a device that was told about it, and it is the only way to
/// know who is owed a notification that no longer names anything at all.
pub fn per_target<T: PartialEq + Clone>(
    change: &Change,
    eligible: &HashMap<String, Vec<T>>,
) -> Vec<(T, Notification)> {
    struct Owed<T> {
        target: T,
        mine: Vec<Blocked>,
        alert: bool,
        /// Whether this change touched anything this device may see. A device whose own set did
        /// not move already has the right notification on its screen, and re-POSTing it is a
        /// wake-up that says nothing.
        affected: bool,
    }
    let mut owed: Vec<Owed<T>> = Vec::new();
    let slot = |owed: &mut Vec<Owed<T>>, target: &T| -> usize {
        match owed.iter().position(|o| &o.target == target) {
            Some(at) => at,
            None => {
                owed.push(Owed {
                    target: target.clone(),
                    mine: Vec::new(),
                    alert: false,
                    affected: false,
                });
                owed.len() - 1
            }
        }
    };

    for pane in &change.outstanding {
        let Some(for_pane) = eligible.get(&pane.pane) else {
            continue;
        };
        for target in for_pane {
            let at = slot(&mut owed, target);
            owed[at].mine.push(pane.clone());
            owed[at].alert |= change.fresh.contains(&pane.pane);
            owed[at].affected |= change.fresh.contains(&pane.pane);
        }
    }
    // A device eligible for a pane that just cleared is exactly a device that was told about it,
    // and is the only device owed a notification that no longer names anything at all.
    for pane in &change.cleared {
        let Some(for_pane) = eligible.get(pane) else {
            continue;
        };
        for target in for_pane {
            let at = slot(&mut owed, target);
            owed[at].affected = true;
        }
    }

    owed.into_iter()
        .filter(|owed| owed.affected)
        .filter_map(|owed| match owed.alert {
            true => Notification::batch(owed.mine).map(|note| (owed.target, note)),
            false => Some((owed.target, Notification::resync(owed.mine))),
        })
        .collect()
}

/// Drives the collection window. Returns the change once one has closed, or `None` when the
/// producer has gone away.
pub async fn collect(rx: &mut mpsc::Receiver<Change>, window: Duration) -> Option<Change> {
    let mut change = rx.recv().await?;
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(next)) => change.absorb(next),
            // The producer is gone, but what it already sent is still worth delivering.
            Ok(None) => break,
            Err(_) => break,
        }
    }
    (!change.is_empty()).then_some(change)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(pane: &str, question: Option<&str>) -> Blocked {
        Blocked {
            pane: pane.into(),
            node: "01J".into(),
            agent: Some("claude".into()),
            label: None,
            question: question.map(str::to_string),
        }
    }

    fn ids(change: &Change) -> Vec<String> {
        change.outstanding.iter().map(|p| p.pane.clone()).collect()
    }

    /// The rule the brief asks for: two panes blocking together are one notification, not a race.
    #[tokio::test(start_paused = true)]
    async fn two_panes_blocking_together_close_one_batch() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(Change::fresh(vec![blocked("01J/w1:p1", Some("Run the tests?"))]))
            .await
            .unwrap();
        tx.send(Change::fresh(vec![
            blocked("01J/w1:p1", Some("Run the tests?")),
            blocked("01J/w2:p1", Some("Apply the patch?")),
        ]))
        .await
        .unwrap();
        let change = collect(&mut rx, WINDOW).await.unwrap();
        assert_eq!(change.outstanding.len(), 2);
        assert_eq!(change.fresh.len(), 2);
        assert_eq!(Notification::batch(change.outstanding).unwrap().count, 2);
    }

    /// And the window has to actually close, or a lone blocked agent waits forever for company
    /// that never comes.
    #[tokio::test(start_paused = true)]
    async fn one_pane_alone_still_closes_its_batch() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(Change::fresh(vec![blocked("01J/w1:p1", None)]))
            .await
            .unwrap();
        assert_eq!(ids(&collect(&mut rx, WINDOW).await.unwrap()), ["01J/w1:p1"]);
    }

    /// A pane whose question is re-read inside one window is one entry, carrying the later
    /// question — the screen is what the question was read from, and the later read saw more of it.
    #[tokio::test(start_paused = true)]
    async fn a_pane_that_blocks_twice_in_one_window_is_one_entry() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(Change::fresh(vec![blocked("01J/w1:p1", None)]))
            .await
            .unwrap();
        tx.send(Change::fresh(vec![blocked("01J/w1:p1", Some("Run the tests?"))]))
            .await
            .unwrap();
        let change = collect(&mut rx, WINDOW).await.unwrap();
        assert_eq!(change.outstanding.len(), 1);
        assert_eq!(change.outstanding[0].question.as_deref(), Some("Run the tests?"));
    }

    /// The whole point of carrying `cleared` through the window: a block and its answer that land
    /// together must still leave the *second* pane named, and must still tell the device that the
    /// first one is gone.
    #[tokio::test(start_paused = true)]
    async fn a_block_and_an_answer_in_one_window_keep_both_halves() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(Change::fresh(vec![blocked("01J/w2:p1", None)]))
            .await
            .unwrap();
        tx.send(Change::cleared(
            vec![blocked("01J/w2:p1", None)],
            ["01J/w1:p1".to_string()],
        ))
        .await
        .unwrap();
        let change = collect(&mut rx, WINDOW).await.unwrap();
        assert_eq!(ids(&change), ["01J/w2:p1"]);
        assert!(change.fresh.contains("01J/w2:p1"));
        assert!(change.cleared.contains("01J/w1:p1"));
    }

    /// A pane that blocked and answered inside one window was never on a phone. Alerting about it
    /// is a buzz for nothing, and clearing it is a payload nobody needed.
    #[tokio::test(start_paused = true)]
    async fn a_pane_that_blocked_and_answered_inside_one_window_is_not_a_notification() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(Change::fresh(vec![blocked("01J/w1:p1", None)]))
            .await
            .unwrap();
        tx.send(Change::cleared(Vec::new(), ["01J/w1:p1".to_string()]))
            .await
            .unwrap();
        assert!(
            collect(&mut rx, WINDOW).await.is_none(),
            "nothing was shown, so there is nothing to alert about or take down"
        );
    }

    /// Muting one agent must not silence the herd, and it must not silence the batch either.
    #[test]
    fn a_muted_pane_is_dropped_from_that_devices_notification_and_no_ones_else() {
        let change = Change::fresh(vec![blocked("01J/w1:p1", None), blocked("01J/w2:p1", None)]);
        let eligible = HashMap::from([
            ("01J/w1:p1".to_string(), vec!["phone", "laptop"]),
            ("01J/w2:p1".to_string(), vec!["laptop"]),
        ]);
        let mut notes = per_target(&change, &eligible);
        notes.sort_by_key(|(target, _)| *target);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].0, "laptop");
        assert_eq!(notes[0].1.count, 2);
        assert_eq!(notes[1].0, "phone");
        assert_eq!(notes[1].1.count, 1);
        assert_eq!(notes[1].1.pane.as_deref(), Some("01J/w1:p1"));
    }

    #[test]
    fn a_pane_nobody_may_receive_produces_no_notification() {
        let change = Change::fresh(vec![blocked("01J/w1:p1", None)]);
        let notes = per_target::<&str>(&change, &HashMap::new());
        assert!(notes.is_empty());
    }

    /// The defect this whole change exists for: answered at the desk, still on the phone.
    #[test]
    fn a_device_told_about_a_pane_that_cleared_is_sent_the_notification_that_takes_it_down() {
        let change = Change::cleared(Vec::new(), ["01J/w1:p1".to_string()]);
        let eligible = HashMap::from([("01J/w1:p1".to_string(), vec!["phone"])]);
        let notes = per_target(&change, &eligible);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "phone");
        assert_eq!(notes[0].1.count, 0);
        assert!(!notes[0].1.alert);
    }

    /// A second agent blocking used to take the first one off the phone: the payload named the
    /// edge, and the tag made it replace everything before it. The payload is the set now.
    #[test]
    fn a_second_agent_blocking_leaves_the_first_one_named() {
        let change = Change {
            outstanding: vec![blocked("01J/w1:p1", None), blocked("01J/w2:p1", None)],
            fresh: HashSet::from(["01J/w2:p1".to_string()]),
            cleared: HashSet::new(),
        };
        let eligible = HashMap::from([
            ("01J/w1:p1".to_string(), vec!["phone"]),
            ("01J/w2:p1".to_string(), vec!["phone"]),
        ]);
        let notes = per_target(&change, &eligible);
        assert_eq!(notes[0].1.count, 2, "the older block is still on the phone");
        assert!(notes[0].1.alert, "the new one is still news");
    }

    /// Answering one of two leaves a notification naming the other — and it must not buzz. A phone
    /// that vibrates to report *less* waiting is a phone that gets muted.
    #[test]
    fn a_shrinking_set_resyncs_without_alerting() {
        let change = Change::cleared(
            vec![blocked("01J/w2:p1", Some("Apply the patch?"))],
            ["01J/w1:p1".to_string()],
        );
        let eligible = HashMap::from([
            ("01J/w1:p1".to_string(), vec!["phone"]),
            ("01J/w2:p1".to_string(), vec!["phone"]),
        ]);
        let notes = per_target(&change, &eligible);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].1.count, 1);
        assert!(!notes[0].1.alert);
    }

    /// The clear is addressed by eligibility, like everything else: the device that was told about
    /// the answered pane gets the payload that takes it down, and a device that never saw it gets
    /// only its own set.
    #[test]
    fn a_clear_reaches_the_device_that_was_told_and_leaves_another_devices_set_alone() {
        let change = Change::cleared(
            vec![blocked("01J/w2:p1", Some("Apply the patch?"))],
            ["01J/w1:p1".to_string()],
        );
        let eligible = HashMap::from([
            ("01J/w1:p1".to_string(), vec!["phone"]),
            ("01J/w2:p1".to_string(), vec!["phone", "laptop"]),
        ]);
        let mut notes = per_target(&change, &eligible);
        notes.sort_by_key(|(target, _)| *target);
        assert_eq!(notes.len(), 1);
        assert_eq!(
            notes[0].0, "phone",
            "the laptop's own set did not move, so waking it says nothing"
        );
        assert_eq!(notes[0].1.count, 1, "the phone keeps the pane it still has");
    }

    /// And a device this change did not touch is not POSTed to at all. Every wake-up costs the
    /// radio and the user's attention; one that repeats what is already on the screen buys neither.
    #[test]
    fn a_device_whose_own_set_did_not_move_is_not_woken() {
        let change = Change::fresh(vec![blocked("01J/w1:p1", None)]);
        let mut with_bystander = change.clone();
        with_bystander.outstanding.push(blocked("01J/w2:p1", None));
        let eligible = HashMap::from([
            ("01J/w1:p1".to_string(), vec!["phone"]),
            ("01J/w2:p1".to_string(), vec!["laptop"]),
        ]);
        let notes = per_target(&with_bystander, &eligible);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "phone");
    }
}
