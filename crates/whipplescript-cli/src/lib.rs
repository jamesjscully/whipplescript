//! Public native integration surface for WhippleScript hosts.
//!
//! The CLI binary and embedding hosts must cross the same governance trust
//! boundary. Keeping these modules in the package library prevents a host from
//! reimplementing envelope parsing, attestation verification, or IFC semantics.

pub mod gov;
pub mod host_policy;
pub mod host_protocol;
pub mod host_runtime;
pub mod ifc;
pub mod principal;

/// Native versioned-workspace substrate used by embedding hosts. Re-exported
/// here so hosts depend on WhippleScript's published integration surface rather
/// than an implementation crate.
pub mod workspace {
    pub use whipplescript_workspace::*;
}
