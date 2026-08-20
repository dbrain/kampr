use crate::note::{Blocked, Notification};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

/// How long a first block waits for company.
///
/// Long enough that three agents finishing a batch of edits arrive as one notification, short
/// enough that a lone blocked agent is not held back noticeably. It is a *collection* window, not
/// a rate limit: the window starts at the first block and does not extend, so a steady trickle
/// still gets a notification every window rather than never.
pub const WINDOW: Duration = Duration::from_millis(900);

/// Collects blocked panes into batches.
///
/// Kept separate from delivery so the batching rule can be tested against a clock rather than
/// against a push service.
#[derive(Debug, Default)]
pub struct Batch {
    panes: Vec<Blocked>,
}

impl Batch {
    /// A pane that blocks twice inside one window — a status flapping, or two sessions reporting
    /// the same herd — is one entry, and the later reading wins because its question is fresher.
    pub fn push(&mut self, blocked: Blocked) {
        match self.panes.iter_mut().find(|p| p.pane == blocked.pane) {
            Some(existing) => *existing = blocked,
            None => self.panes.push(blocked),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    pub fn take(&mut self) -> Vec<Blocked> {
        std::mem::take(&mut self.panes)
    }
}

/// One batch, split by who may receive which pane.
///
/// A device that muted one of three blocked agents must see a notification naming the other two,
/// not the whole batch and not nothing. So the split is per subscription and the notification is
/// built from that subscription's own eligible panes.
pub fn per_target<T: PartialEq + Clone>(
    panes: &[Blocked],
    eligible: &HashMap<String, Vec<T>>,
) -> Vec<(T, Notification)> {
    let mut by_target: Vec<(T, Vec<Blocked>)> = Vec::new();
    for pane in panes {
        let Some(targets) = eligible.get(&pane.pane) else {
            continue;
        };
        for target in targets {
            match by_target.iter_mut().find(|(t, _)| t == target) {
                Some((_, collected)) => collected.push(pane.clone()),
                None => by_target.push((target.clone(), vec![pane.clone()])),
            }
        }
    }
    by_target
        .into_iter()
        .filter_map(|(target, panes)| Notification::batch(panes).map(|note| (target, note)))
        .collect()
}

/// Drives the collection window. Returns the batch once one has closed, or `None` when the
/// producer has gone away.
pub async fn collect(rx: &mut mpsc::Receiver<Blocked>, window: Duration) -> Option<Vec<Blocked>> {
    let mut batch = Batch::default();
    batch.push(rx.recv().await?);
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(blocked)) => batch.push(blocked),
            // The producer is gone, but what it already sent is still worth delivering.
            Ok(None) => break,
            Err(_) => break,
        }
    }
    Some(batch.take())
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

    /// The rule the brief asks for: two panes blocking together are one notification, not a race.
    #[tokio::test(start_paused = true)]
    async fn two_panes_blocking_together_close_one_batch() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(blocked("01J/w1:p1", Some("Run the tests?")))
            .await
            .unwrap();
        tx.send(blocked("01J/w2:p1", Some("Apply the patch?")))
            .await
            .unwrap();
        let batch = collect(&mut rx, WINDOW).await.unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(Notification::batch(batch).unwrap().count, 2);
    }

    /// And the window has to actually close, or a lone blocked agent waits forever for company
    /// that never comes.
    #[tokio::test(start_paused = true)]
    async fn one_pane_alone_still_closes_its_batch() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(blocked("01J/w1:p1", None)).await.unwrap();
        let batch = collect(&mut rx, WINDOW).await.unwrap();
        assert_eq!(batch.len(), 1);
    }

    /// A pane whose status flaps inside one window is one entry, carrying the later question —
    /// the screen is what the question was read from, and the later read saw more of it.
    #[tokio::test(start_paused = true)]
    async fn a_pane_that_blocks_twice_in_one_window_is_one_entry() {
        let (tx, mut rx) = mpsc::channel(8);
        tx.send(blocked("01J/w1:p1", None)).await.unwrap();
        tx.send(blocked("01J/w1:p1", Some("Run the tests?")))
            .await
            .unwrap();
        let batch = collect(&mut rx, WINDOW).await.unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].question.as_deref(), Some("Run the tests?"));
    }

    /// Muting one agent must not silence the herd, and it must not silence the batch either.
    #[test]
    fn a_muted_pane_is_dropped_from_that_devices_notification_and_no_ones_else() {
        let panes = vec![blocked("01J/w1:p1", None), blocked("01J/w2:p1", None)];
        let eligible = HashMap::from([
            ("01J/w1:p1".to_string(), vec!["phone", "laptop"]),
            ("01J/w2:p1".to_string(), vec!["laptop"]),
        ]);
        let mut notes = per_target(&panes, &eligible);
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
        let panes = vec![blocked("01J/w1:p1", None)];
        let notes = per_target::<&str>(&panes, &HashMap::new());
        assert!(notes.is_empty());
    }
}
