//! Host-neutral effect configuration (DR-0033 chunk 4).
//!
//! The native effect executor drives handlers with a `WorkerOptions` carrying
//! native-only concerns (exec profile, provider config paths, script manifest,
//! work-unit root, fixture-outcome maps). A durable-object handler needs none of
//! that — only the small, host-neutral surface a handler actually reads. This is
//! that surface: the native CLI projects its `WorkerOptions` into an
//! `EffectConfig`, and the DO builds one from its secrets/bindings, so the
//! generic handler cores depend on neither host's option struct and can live in
//! the wasm-clean kernel.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EffectConfig {
    /// The provider name recorded on runs/terminals (e.g. `"fixture"`, a
    /// configured provider id).
    pub provider: String,
    /// Whether the fixture executor should settle this effect to a failure
    /// (the native `--fail` knob / per-agent fixture outcome). Real-provider
    /// execution ignores it.
    pub outcome_failed: bool,
}
