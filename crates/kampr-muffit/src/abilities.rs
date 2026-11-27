use crate::wire::BASELINE;
use kampr_core::conversation::Abilities;
use kampr_core::conversation::abilities::{AskAbility, Fact, InputAbility, Lifecycle, Steer, Stream};

/// The minor each capability arrived in (`../muffit/docs/api/kampr.md`, conv-sources).
pub const QUESTION: u32 = 304;
pub const SUB_LIVE: u32 = 305;
pub const SHELL_TASKS: u32 = 306;
pub const VERBS: u32 = 307;
pub const CATALOG: u32 = 308;
pub const BILLING: u32 = 309;

/// What a pane can draw for a daemon that greeted with `version`: nothing it cannot serve, so a
/// 3.3 daemon is exactly the pane it always was.
pub fn abilities(version: u32) -> Abilities {
    let mut a = Abilities::default();
    a.input.insert(InputAbility::Images);
    a.steer.extend([
        Steer::Interrupt,
        Steer::Continue,
        Steer::Pause,
        Steer::Resume,
        Steer::Stop,
        Steer::Compact,
    ]);
    a.asks.extend([AskAbility::Permission, AskAbility::Always]);
    a.streams.extend([Stream::Text, Stream::TaskOutput]);
    a.facts.extend([Fact::Context, Fact::Cost, Fact::Trouble]);
    a.lifecycle
        .extend([Lifecycle::Create, Lifecycle::Resume, Lifecycle::End]);
    let at = version.max(BASELINE);
    if at >= QUESTION {
        a.asks.extend([AskAbility::Question, AskAbility::FreeText]);
    }
    if at >= SUB_LIVE {
        a.streams.insert(Stream::SubLive);
    }
    if at >= SHELL_TASKS {
        a.steer.insert(Steer::StopTask);
    }
    if at >= VERBS {
        a.streams.insert(Stream::ToolArgs);
        a.steer.extend([Steer::Rename, Steer::Clear, Steer::Fork]);
        a.input.insert(InputAbility::Unqueue);
    }
    if at >= CATALOG {
        a.input.insert(InputAbility::Slash);
    }
    if at >= BILLING {
        a.facts.extend([Fact::Billing, Fact::Limits]);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_minor_adds_only_what_it_serves() {
        type Has = Box<dyn Fn(&Abilities) -> bool>;
        let cases: [(u32, Has); 6] = [
            (QUESTION, Box::new(|a| a.asks.contains(&AskAbility::Question))),
            (SUB_LIVE, Box::new(|a| a.streams.contains(&Stream::SubLive))),
            (SHELL_TASKS, Box::new(|a| a.steer.contains(&Steer::StopTask))),
            (
                VERBS,
                Box::new(|a| a.streams.contains(&Stream::ToolArgs) && a.steer.contains(&Steer::Rename)),
            ),
            (CATALOG, Box::new(|a| a.input.contains(&InputAbility::Slash))),
            (
                BILLING,
                Box::new(|a| a.facts.contains(&Fact::Billing) && a.facts.contains(&Fact::Limits)),
            ),
        ];
        for (since, has) in cases {
            assert!(!has(&abilities(since - 1)), "declared before {since}");
            assert!(has(&abilities(since)), "not declared at {since}");
        }
        assert_eq!(abilities(0), abilities(BASELINE));
    }
}
