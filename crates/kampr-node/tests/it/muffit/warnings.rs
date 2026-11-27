//! The warnings the node logs while a test runs: `capture` holds them for as long as its guard lives.

use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Default)]
pub struct Warnings(Arc<Mutex<Vec<String>>>);

struct Say(String);

impl tracing::field::Visit for Say {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!(" {}={value:?}", field.name()));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut say = Say(String::new());
            event.record(&mut say);
            self.0.lock().unwrap().push(say.0);
        }
    }
}

impl Warnings {
    pub fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let warnings = Self::default();
        let guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(warnings.clone()));
        (warnings, guard)
    }

    pub fn naming(&self, words: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains(words))
            .cloned()
            .collect()
    }
}
