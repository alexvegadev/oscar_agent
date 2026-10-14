//! Bounded desktop conversations and opt-in registry-backed autonomous runs.
pub mod messages;
pub mod runtime;
pub mod session;
pub mod settings;

#[cfg(feature = "desktop")]
pub mod desktop;
