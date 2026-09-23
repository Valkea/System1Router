//! Shared routing core for the `system1router` executable and host adapters.

pub mod adapters;
pub mod classification;
pub mod consent;
pub mod config;
pub mod gateway;
pub mod policy;
pub mod protocol;
pub mod redaction;
pub mod runtime;
pub mod telemetry;
pub mod transaction;

pub use classification::{DecisionProvider, ProviderError, ProviderHealth, ProviderInput};
pub use config::Config;
pub use protocol::*;
pub use runtime::process_event;
