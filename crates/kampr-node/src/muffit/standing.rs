//! What `kampr doctor` says of the muffit daemon this node is configured to dial.

use crate::config::Config;
use crate::config::muffit::UrlFrom;
use kampr_muffit::{Client, Reach};

pub struct Standing {
    pub url: String,
    pub from: UrlFrom,
    pub verdict: Verdict,
}

pub enum Verdict {
    Answers,
    Away(String),
    Refused { status: u16, why: String },
}

/// `None` when this node does not talk to a muffit daemon at all.
pub async fn standing(config: &Config) -> Option<Standing> {
    let endpoint = config.muffit.resolve(&config.journal_home())?;
    let reach = match Client::new(&endpoint.url, &endpoint.token) {
        Ok(client) => client.reach().await,
        Err(e) => Reach::Away(format!("{e:#}")),
    };
    let verdict = match reach {
        Reach::Answers => Verdict::Answers,
        Reach::Away(why) => Verdict::Away(why),
        Reach::Refused(refusal) => Verdict::Refused {
            status: refusal.status,
            why: refusal.why,
        },
    };
    Some(Standing {
        url: endpoint.url,
        from: endpoint.from,
        verdict,
    })
}
