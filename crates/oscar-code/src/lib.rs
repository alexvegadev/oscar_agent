//! Bounded, in-memory desktop proposal sessions. Model output never executes tools.
pub mod session;
pub mod settings;

#[cfg(feature = "desktop")]
pub mod desktop;
