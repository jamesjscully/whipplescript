-- Context-assembly Phase 2 (Decision 3): skills carry their SKILL.md body in the
-- store so an activation read resolves identical bytes on native and on the
-- durable object (no filesystem there), replay-stable. `content_hash` becomes the
-- hash of this body, so evidence pins the exact bytes the model was shown and the
-- loader can detect an unchanged vs. drifted skill.
ALTER TABLE skills ADD COLUMN body TEXT NOT NULL DEFAULT '';
