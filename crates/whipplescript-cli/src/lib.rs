//! Public native integration surface for WhippleScript hosts.
//!
//! The CLI binary and embedding hosts must cross the same governance trust
//! boundary. Keeping these modules in the package library prevents a host from
//! reimplementing envelope parsing, attestation verification, or IFC semantics.

pub use whipplescript_kernel::{gov, host_policy, host_protocol, ifc, principal};
pub mod host_runtime;
