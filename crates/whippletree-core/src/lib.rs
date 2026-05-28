//! Shared types for the Whippletree rule-machine runtime.

/// Current implementation stage for the active redesign.
pub const IMPLEMENTATION_STAGE: &str = "stage-0-skeleton";

/// Returns the workspace package version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_stage_marker() {
        assert_eq!(IMPLEMENTATION_STAGE, "stage-0-skeleton");
    }

    #[test]
    fn exposes_version() {
        assert!(!version().is_empty());
    }
}
