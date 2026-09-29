//! Shared routing core for the `system1router` executable and host adapters.

pub mod adapters;
pub mod claude_code;
pub mod classification;
pub mod config;
pub mod gateway;
pub mod host_install;
pub mod policy;
pub mod protocol;
pub mod redaction;
pub mod runtime;
pub mod telemetry;

pub use classification::{DecisionProvider, ProviderError, ProviderInput};
pub use config::Config;
pub use protocol::*;
pub use runtime::process_event;
