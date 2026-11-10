use kampr_journal::{Block, Change, LIVE_ID, Role, Turn};
use std::collections::VecDeque;

const RECENT: usize = 8;

#[derive(Default)]
pub struct HookedLive {
    sent: Option<String>,
    recorded: VecDeque<String>,
}

impl HookedLive {
    pub fn record(&mut self, turns: &[Turn]) {
        for turn in turns
            .iter()
            .filter(|t| t.role == Role::Assistant && t.id != LIVE_ID)
        {
            let text: String = turn
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Md { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                continue;
            }
            self.recorded.push_back(fingerprint(&text));
            if self.recorded.len() > RECENT {
                self.recorded.pop_front();
            }
        }
    }

    pub fn superseded(&mut self) -> bool {
        match self.sent.as_deref() {
            Some(sent) if self.known(sent) => {
                self.sent = None;
                true
            }
            _ => false,
        }
    }

    pub fn next(&mut self, live: Option<String>) -> Change {
        let live = live.filter(|text| !self.known(text));
        if live == self.sent {
            return Change::Held;
        }
        self.sent = live.clone();
        match live {
            Some(text) => {
                let mut turn = Turn::new(LIVE_ID, Role::Assistant, None);
                turn.blocks.push(Block::md(text));
                Change::Show(turn)
            }
            None => Change::Retire,
        }
    }

    fn known(&self, text: &str) -> bool {
        let live = fingerprint(text);
        !live.is_empty() && self.recorded.iter().any(|known| known.contains(&live))
    }
}

fn fingerprint(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
