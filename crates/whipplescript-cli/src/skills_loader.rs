//! Skill registry loader (context-assembly tracker Phase 2, item 1).
//!
//! Walks a skills directory — each skill in its own subdirectory containing a
//! `SKILL.md` — validates the agentskills.io frontmatter, and registers each skill
//! into the store with its **content-addressed body** (the full `SKILL.md` bytes
//! the model reads on activation). Load order is deterministic (sorted by
//! directory name) so the catalogue and its evidence are stable.
//!
//! Per Decision 4, a skill's `allowed-tools` is stored as provenance metadata only
//! and never widens tool authority.

use std::path::Path;

use serde_json::{json, Map, Value};
use whipplescript_store::skill_frontmatter::parse_skill_frontmatter;
use whipplescript_store::{SkillRegistration, SqliteStore};

/// A skill that was ingested into the store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedSkill {
    pub name: String,
    pub version: String,
    pub source_path: String,
}

/// Load every `<dir>/<name>/SKILL.md` under `skills_dir` into `store`, tagged with
/// `source` (e.g. `"workspace"`, `"builtin"`). Returns the loaded skills in
/// deterministic order. A subdirectory without a `SKILL.md` is skipped; a present
/// but invalid `SKILL.md` is a hard error naming the offending skill.
pub fn load_skills_from_dir(
    store: &SqliteStore,
    skills_dir: &Path,
    source: &str,
) -> Result<Vec<LoadedSkill>, String> {
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }

    // Collect skill subdirectories in a deterministic (sorted) order.
    let mut dirs: Vec<_> = std::fs::read_dir(skills_dir)
        .map_err(|error| format!("cannot read skills directory {skills_dir:?}: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    let mut loaded = Vec::new();
    for dir in dirs {
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let dir_name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("skill directory {dir:?} has a non-UTF-8 name"))?;
        let body = std::fs::read_to_string(&skill_md)
            .map_err(|error| format!("cannot read {skill_md:?}: {error}"))?;
        let frontmatter = parse_skill_frontmatter(&body)
            .map_err(|error| format!("invalid skill `{dir_name}` ({skill_md:?}): {error}"))?;
        if frontmatter.name != dir_name {
            return Err(format!(
                "skill `name` `{}` does not match its directory `{dir_name}` ({skill_md:?})",
                frontmatter.name
            ));
        }

        // The frontmatter `metadata` map plus the provenance-only license /
        // compatibility / allowed-tools become the skill's metadata JSON.
        let mut metadata: Map<String, Value> = frontmatter.metadata.clone();
        if let Some(license) = &frontmatter.license {
            metadata.insert("license".to_owned(), json!(license));
        }
        if let Some(compatibility) = &frontmatter.compatibility {
            metadata.insert("compatibility".to_owned(), json!(compatibility));
        }
        if !frontmatter.allowed_tools.is_empty() {
            metadata.insert("allowed_tools".to_owned(), json!(frontmatter.allowed_tools));
        }
        let version = frontmatter
            .metadata
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.0.0")
            .to_owned();
        let metadata_json = Value::Object(metadata).to_string();
        let source_path = skill_md.to_string_lossy().into_owned();
        let skill_id = format!("skill:{}", frontmatter.name);

        store
            .register_skill(SkillRegistration {
                skill_id: &skill_id,
                name: &frontmatter.name,
                version: &version,
                source,
                source_path: &source_path,
                body: &body,
                description: &frontmatter.description,
                required_capabilities_json: "[]",
                metadata_json: &metadata_json,
            })
            .map_err(|error| format!("cannot register skill `{}`: {error:?}", frontmatter.name))?;

        loaded.push(LoadedSkill {
            name: frontmatter.name,
            version,
            source_path,
        });
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, name: &str, frontmatter_name: &str, body_line: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("create skill dir");
        let content = format!("---\nname: {frontmatter_name}\ndescription: A {name} skill.\n---\n# {name}\n{body_line}\n");
        std::fs::write(dir.join("SKILL.md"), content).expect("write SKILL.md");
    }

    #[test]
    fn loads_skills_deterministically_with_content_addressed_bodies() {
        let tmp = std::env::temp_dir().join(format!("whip-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        write_skill(&tmp, "beta", "beta", "second");
        write_skill(&tmp, "alpha", "alpha", "first");
        // A directory with no SKILL.md is skipped.
        std::fs::create_dir_all(tmp.join("empty")).expect("create empty dir");

        let store = SqliteStore::open_in_memory().expect("store");
        let loaded = load_skills_from_dir(&store, &tmp, "workspace").expect("loads");
        let names: Vec<&str> = loaded.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]); // sorted, empty dir skipped

        let skills = store.list_skills().expect("list");
        assert_eq!(skills.len(), 2);
        // content_hash addresses the body, so the two distinct bodies differ.
        assert_ne!(skills[0].content_hash, skills[1].content_hash);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_name_mismatch_and_bad_frontmatter() {
        let tmp = std::env::temp_dir().join(format!("whip-skills-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        write_skill(&tmp, "mismatch", "other-name", "x");
        let store = SqliteStore::open_in_memory().expect("store");
        assert!(load_skills_from_dir(&store, &tmp, "workspace").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
