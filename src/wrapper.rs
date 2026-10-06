//! Wrappers for different plugin types. Each wrapper has an entry point macro that you can pass the
//! name of a type that implements `Plugin` to. The macro will handle the rest.

pub mod clap;
pub(crate) mod state;
pub(crate) mod util;

#[cfg(feature = "standalone")]
pub mod standalone;
#[cfg(feature = "vst3")]
pub mod vst3;

// MXM PATCH (defect 11): an infallible persistent field can explicitly distinguish successful
// canonicalization from rejection during the wrapper's scoped restore validation.
pub use state::accept_canonicalized_persistent_field;

// This is used by the wrappers.
pub use util::setup_logger;
