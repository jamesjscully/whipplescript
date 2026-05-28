//! Source parser scaffold for `.armature` programs.

/// Placeholder parse entry point.
///
/// The real parser will return a recoverable syntax tree with source spans.
pub fn parser_stage() -> &'static str {
    armature_core::IMPLEMENTATION_STAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_scaffold_links_to_core() {
        assert_eq!(parser_stage(), "stage-0-skeleton");
    }
}
