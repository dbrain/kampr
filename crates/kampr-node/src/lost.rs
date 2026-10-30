//! Holds this node's viewers were given and somebody else took, from wherever they were taken.

use crate::state::Node;
use tokio::sync::broadcast::{self, error::RecvError};

pub enum Lost {
    Herdr { local: String, token: u64 },
    Native { local: String, token: u64 },
    Peer { global: String, token: u64 },
}

pub struct LostFeed {
    herdr: Option<broadcast::Receiver<(String, u64)>>,
    native: Option<broadcast::Receiver<(String, u64)>>,
    peers: Option<broadcast::Receiver<(String, u64)>>,
}

impl LostFeed {
    pub fn subscribe(node: &Node) -> Self {
        Self {
            herdr: Some(node.holds.subscribe_lost()),
            native: node.primary().native.as_ref().map(|n| n.subscribe_lost()),
            peers: Some(node.peers.subscribe_lost()),
        }
    }

    /// A lagged feed skips what it missed: a viewer that is not told keeps asking on its next look,
    /// which is where it was before any of this existed.
    pub async fn recv(&mut self) -> Lost {
        loop {
            tokio::select! {
                Some(got) = next(&mut self.herdr) => {
                    if let Some((local, token)) = got {
                        return Lost::Herdr { local, token };
                    }
                }
                Some(got) = next(&mut self.native) => {
                    if let Some((local, token)) = got {
                        return Lost::Native { local, token };
                    }
                }
                Some(got) = next(&mut self.peers) => {
                    if let Some((global, token)) = got {
                        return Lost::Peer { global, token };
                    }
                }
                else => std::future::pending::<()>().await,
            }
        }
    }
}

/// `None` once the feed has closed, `Some(None)` for a lag.
async fn next(feed: &mut Option<broadcast::Receiver<(String, u64)>>) -> Option<Option<(String, u64)>> {
    let rx = feed.as_mut()?;
    match rx.recv().await {
        Ok(got) => Some(Some(got)),
        Err(RecvError::Lagged(_)) => Some(None),
        Err(RecvError::Closed) => {
            *feed = None;
            None
        }
    }
}
