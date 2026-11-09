use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    Muffit,
    Containers,
    Stopped,
}

impl Fold {
    fn key(self) -> &'static str {
        match self {
            Self::Muffit => "muffit",
            Self::Containers => "containers",
            Self::Stopped => "stopped",
        }
    }

    fn open_by_default(self) -> bool {
        matches!(self, Self::Muffit)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Folds(BTreeMap<String, BTreeMap<String, bool>>);

impl Folds {
    pub fn open(&self, node: &str, fold: Fold) -> bool {
        self.0
            .get(node)
            .and_then(|sections| sections.get(fold.key()))
            .copied()
            .unwrap_or(fold.open_by_default())
    }

    pub fn flip(&mut self, node: &str, fold: Fold) {
        let open = !self.open(node, fold);
        self.0
            .entry(node.to_string())
            .or_default()
            .insert(fold.key().to_string(), open);
    }
}
