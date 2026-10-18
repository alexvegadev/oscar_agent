//! Experimental, opt-in durable tracing. Content capture is never implicit.
mod journal;
pub mod provider;
pub use journal::{
    AttemptOrigin, CaptureMode, Journal, JournalEvent, JournalLimits, JournalRecord, Payload,
    SessionId, Verification, verify,
};
