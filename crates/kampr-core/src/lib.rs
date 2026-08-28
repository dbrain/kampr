//! Node core: the provider seam, one emulator per pane, and the values that go on the wire.
//!
//! Two probed constraints shape everything here. Kampr never resizes a pane, so rendering is an
//! `observe` stream at the pane's *native* geometry and input goes over the JSON API
//! (probe #17/#18). And frames carry end state only, so scrollback comes from `pane.read recent`
//! and never from the stream (probe #25).

pub mod agent_view;
pub mod backoff;
pub mod herdr_provider;
pub mod naming;
pub mod provider;
pub mod registry;
pub mod reporter;
pub mod scrollback;
pub mod wire;

pub use backoff::Backoff;
pub use herdr_provider::{HerdrConfig, HerdrProvider};
pub use naming::{DEFAULT_TEMPLATE, Fields, Template, TemplateError};
pub use provider::{Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
pub use registry::{PaneRegistry, PaneUpdate, Watcher};
pub use reporter::{Reported, Reporter};
pub use scrollback::ScrollbackDoc;
pub use wire::{ClientMsg, Encoder, ServerMsg};
