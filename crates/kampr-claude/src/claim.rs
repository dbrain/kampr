use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use kampr_core::conversation::Refusal;

/// The Claude sessions being moved between a headless child and a terminal. From the check that
/// nothing holds a transcript until the new writer starts, neither a herd entry nor an open file
/// stands for it, and a second writer forks it (#655).
#[derive(Debug, Default, Clone)]
pub(crate) struct Claims(Arc<Mutex<HashSet<String>>>);

/// One session held until it is dropped.
#[derive(Debug)]
pub struct Claim {
    id: String,
    claims: Claims,
}

impl Claims {
    pub fn take(&self, id: &str) -> Result<Claim, Refusal> {
        if !self.0.lock().unwrap().insert(id.to_string()) {
            return Err(Refusal::new(format!(
                "{id} is being moved already; try again once that has finished"
            )));
        }
        Ok(Claim {
            id: id.to_string(),
            claims: self.clone(),
        })
    }
}

impl Claim {
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.claims.0.lock().unwrap().remove(&self.id);
    }
}
