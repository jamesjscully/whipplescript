//! Durable store scaffold for event logs, facts, effects, and evidence.

/// Placeholder store entry point.
///
/// The real store will be SQLite-backed and replayable from the event log.
pub fn store_stage() -> &'static str {
    armature_core::IMPLEMENTATION_STAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_scaffold_links_to_core() {
        assert_eq!(store_stage(), "stage-0-skeleton");
    }
}
