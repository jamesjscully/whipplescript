//! Deterministic runtime kernel scaffold.

/// Placeholder kernel entry point.
///
/// The real kernel will own rule commits, effect graph enqueueing, dependency
/// release, leases, retries, and trace emission.
pub fn kernel_stage() -> &'static str {
    armature_store::store_stage()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_scaffold_links_to_store() {
        assert_eq!(kernel_stage(), armature_core::IMPLEMENTATION_STAGE);
    }
}
