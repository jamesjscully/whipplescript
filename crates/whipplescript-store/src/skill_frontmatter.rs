//! agentskills.io `SKILL.md` frontmatter parsing + validation (context-assembly
//! tracker Phase 2, item 2).
//!
//! A `SKILL.md` opens with a `---`-fenced YAML frontmatter block. The owned
//! harness validates and stores each skill's `name`/`description` (plus optional
//! `license`/`compatibility`/`metadata`/`allowed-tools`) alongside a
//! content-addressed body. This is a deliberately small, dependency-free subset of
//! YAML — the shape agentskills.io frontmatter actually uses:
//!
//! - scalar values (`key: value`, optionally quoted, or a `>-`/`>`/`|`/`|-` block
//!   scalar spanning following indented lines);
//! - an `allowed-tools` list, either inline flow (`[Read, Write]`) or a block list
//!   (`- Read` lines);
//! - a flat `metadata` map (indented `key: value` scalar entries).
//!
//! Per Decision 4 (and the `skills-never-grant` model), `allowed-tools` is parsed
//! as provenance only — this module never grants authority; policy always decides
//! tool availability.

use serde_json::{Map, Value};

/// The maximum `name` length (agentskills.io).
pub const MAX_NAME_LEN: usize = 64;
/// The maximum `description` length (agentskills.io).
pub const MAX_DESCRIPTION_LEN: usize = 1024;

/// The validated frontmatter of a `SKILL.md`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    /// Recorded as provenance only — never widens tool authority (Decision 4).
    pub allowed_tools: Vec<String>,
    /// Arbitrary flat metadata map (agentskills.io `metadata:`).
    pub metadata: Map<String, Value>,
}

/// Validate a skill `name` against the agentskills.io rules: 1..=64 chars, only
/// `[a-z0-9-]`, no leading/trailing hyphen, no consecutive hyphens. Directory
/// matching is the loader's job (it knows the directory); this validates format.
pub fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("skill `name` must not be empty".to_owned());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "skill `name` must be at most {MAX_NAME_LEN} characters, got {}",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(format!(
            "skill `name` `{name}` must contain only lowercase letters, digits, and hyphens"
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(format!(
            "skill `name` `{name}` must not start or end with a hyphen"
        ));
    }
    if name.contains("--") {
        return Err(format!(
            "skill `name` `{name}` must not contain consecutive hyphens"
        ));
    }
    Ok(())
}

/// Parse and validate the frontmatter block of a `SKILL.md` source.
pub fn parse_skill_frontmatter(source: &str) -> Result<SkillFrontmatter, String> {
    let block = extract_frontmatter_block(source)?;
    let entries = parse_block(&block)?;

    let mut frontmatter = SkillFrontmatter::default();
    let mut seen_name = false;
    let mut seen_description = false;

    for entry in entries {
        match entry.key.as_str() {
            "name" => {
                let name = entry.expect_scalar()?;
                validate_skill_name(&name)?;
                frontmatter.name = name;
                seen_name = true;
            }
            "description" => {
                let description = entry.expect_scalar()?;
                if description.is_empty() {
                    return Err("skill `description` must not be empty".to_owned());
                }
                if description.chars().count() > MAX_DESCRIPTION_LEN {
                    return Err(format!(
                        "skill `description` must be at most {MAX_DESCRIPTION_LEN} characters"
                    ));
                }
                frontmatter.description = description;
                seen_description = true;
            }
            "license" => frontmatter.license = Some(entry.expect_scalar()?),
            "compatibility" => frontmatter.compatibility = Some(entry.expect_scalar()?),
            "allowed-tools" => frontmatter.allowed_tools = entry.expect_list()?,
            "metadata" => frontmatter.metadata = entry.expect_map()?,
            other => {
                return Err(format!(
                    "unknown skill frontmatter field `{other}` (allowed: name, description, \
                     license, compatibility, allowed-tools, metadata)"
                ))
            }
        }
    }

    if !seen_name {
        return Err("skill frontmatter is missing required field `name`".to_owned());
    }
    if !seen_description {
        return Err("skill frontmatter is missing required field `description`".to_owned());
    }
    Ok(frontmatter)
}

/// The raw value shape of a top-level frontmatter entry.
enum RawValue {
    Scalar(String),
    List(Vec<String>),
    Map(Map<String, Value>),
}

struct Entry {
    key: String,
    value: RawValue,
}

impl Entry {
    fn expect_scalar(self) -> Result<String, String> {
        match self.value {
            RawValue::Scalar(value) => Ok(value),
            _ => Err(format!(
                "skill frontmatter field `{}` must be a scalar",
                self.key
            )),
        }
    }

    fn expect_list(self) -> Result<Vec<String>, String> {
        match self.value {
            RawValue::List(items) => Ok(items),
            // A single scalar is tolerated as a one-element list.
            RawValue::Scalar(value) if !value.is_empty() => Ok(vec![value]),
            _ => Err(format!(
                "skill frontmatter field `{}` must be a list",
                self.key
            )),
        }
    }

    fn expect_map(self) -> Result<Map<String, Value>, String> {
        match self.value {
            RawValue::Map(map) => Ok(map),
            _ => Err(format!(
                "skill frontmatter field `{}` must be a map",
                self.key
            )),
        }
    }
}

/// Extract the text between the opening `---` fence and the next `---` line.
fn extract_frontmatter_block(source: &str) -> Result<String, String> {
    let mut lines = source.lines();
    match lines.next() {
        Some(first) if first.trim() == "---" => {}
        _ => return Err("SKILL.md must open with a `---` frontmatter fence".to_owned()),
    }
    let mut block = String::new();
    for line in lines {
        if line.trim() == "---" {
            return Ok(block);
        }
        block.push_str(line);
        block.push('\n');
    }
    Err("SKILL.md frontmatter is missing its closing `---` fence".to_owned())
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Parse the frontmatter block into top-level entries. Lines are grouped by
/// indentation: a zero-indent `key:` starts an entry, and following more-indented
/// lines (or an inline block-scalar/flow value) form its value.
fn parse_block(block: &str) -> Result<Vec<Entry>, String> {
    let lines: Vec<&str> = block
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .collect();
    let mut entries: Vec<Entry> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if indent_of(line) != 0 {
            return Err(format!(
                "unexpected indented frontmatter line: `{}`",
                line.trim()
            ));
        }
        let (key, inline) = split_key(line)?;
        index += 1;
        // Gather the child lines that are indented under this key.
        let mut children: Vec<&str> = Vec::new();
        while index < lines.len() && indent_of(lines[index]) > 0 {
            children.push(lines[index]);
            index += 1;
        }
        let value = parse_value(&key, inline, &children)?;
        entries.push(Entry { key, value });
    }
    Ok(entries)
}

/// Split a `key: rest` line into the key and the inline remainder (may be empty).
fn split_key(line: &str) -> Result<(String, &str), String> {
    let trimmed = line.trim_end();
    let colon = trimmed.find(':').ok_or_else(|| {
        format!("malformed frontmatter line (expected `key: value`): `{trimmed}`")
    })?;
    let key = trimmed[..colon].trim().to_owned();
    if key.is_empty() {
        return Err(format!(
            "malformed frontmatter line (empty key): `{trimmed}`"
        ));
    }
    let rest = trimmed[colon + 1..].trim_start();
    Ok((key, rest))
}

fn parse_value(key: &str, inline: &str, children: &[&str]) -> Result<RawValue, String> {
    // Block scalar: `>-`, `>`, `|`, `|-` — join the child lines' text.
    if matches!(inline, ">-" | ">" | "|" | "|-" | ">+" | "|+") {
        let folded = inline.starts_with('>');
        return Ok(RawValue::Scalar(join_block_scalar(children, folded)));
    }
    // Inline flow list: `[a, b, c]`.
    if inline.starts_with('[') {
        return Ok(RawValue::List(parse_flow_list(inline)?));
    }
    // Inline scalar present → that is the value; children are unexpected.
    if !inline.is_empty() {
        if !children.is_empty() {
            return Err(format!(
                "frontmatter field `{key}` has both an inline value and indented lines"
            ));
        }
        return Ok(RawValue::Scalar(unquote(inline)));
    }
    // No inline value: children decide list vs map.
    if children.is_empty() {
        return Ok(RawValue::Scalar(String::new()));
    }
    if children
        .iter()
        .all(|line| line.trim_start().starts_with("- "))
    {
        let items = children
            .iter()
            .map(|line| unquote(line.trim_start().trim_start_matches("- ").trim()))
            .filter(|item| !item.is_empty())
            .collect();
        return Ok(RawValue::List(items));
    }
    // Otherwise a flat map of `key: value` scalars.
    let mut map = Map::new();
    for child in children {
        let (child_key, child_inline) = split_key(child)?;
        map.insert(child_key, Value::String(unquote(child_inline)));
    }
    Ok(RawValue::Map(map))
}

/// Join a block scalar's child lines. Folded (`>`) joins with spaces; literal
/// (`|`) preserves line breaks. Both are trimmed of surrounding whitespace.
fn join_block_scalar(children: &[&str], folded: bool) -> String {
    let sep = if folded { " " } else { "\n" };
    children
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(sep)
        .trim()
        .to_owned()
}

fn parse_flow_list(inline: &str) -> Result<Vec<String>, String> {
    let inner = inline
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| format!("malformed inline list: `{inline}`"))?;
    Ok(inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect())
}

/// Strip a single layer of matching single or double quotes.
fn unquote(value: &str) -> String {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return value[1..value.len() - 1].to_owned();
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_first_party_folded_description() {
        let source = "---\nname: whipplescript-author\ndescription: >-\n  Use this skill when\n  authoring workflows.\n---\n# body\n";
        let fm = parse_skill_frontmatter(source).expect("parses");
        assert_eq!(fm.name, "whipplescript-author");
        assert_eq!(fm.description, "Use this skill when authoring workflows.");
        assert!(fm.allowed_tools.is_empty());
    }

    #[test]
    fn parses_optional_fields_list_and_map() {
        let source = "---\nname: demo\ndescription: A demo skill.\nlicense: MIT\ncompatibility: \">=0.2\"\nallowed-tools: [Read, Write]\nmetadata:\n  author: jane\n  tier: gold\n---\n";
        let fm = parse_skill_frontmatter(source).expect("parses");
        assert_eq!(fm.license.as_deref(), Some("MIT"));
        assert_eq!(fm.compatibility.as_deref(), Some(">=0.2"));
        assert_eq!(fm.allowed_tools, vec!["Read", "Write"]);
        assert_eq!(
            fm.metadata.get("author"),
            Some(&Value::String("jane".into()))
        );
    }

    #[test]
    fn parses_block_list_allowed_tools() {
        let source =
            "---\nname: demo\ndescription: A demo.\nallowed-tools:\n  - Read\n  - Grep\n---\n";
        let fm = parse_skill_frontmatter(source).expect("parses");
        assert_eq!(fm.allowed_tools, vec!["Read", "Grep"]);
    }

    #[test]
    fn rejects_bad_name_and_missing_fields() {
        assert!(parse_skill_frontmatter("---\nname: Bad_Name\ndescription: x\n---\n").is_err());
        assert!(parse_skill_frontmatter("---\nname: a--b\ndescription: x\n---\n").is_err());
        assert!(parse_skill_frontmatter("---\nname: -lead\ndescription: x\n---\n").is_err());
        assert!(parse_skill_frontmatter("---\nname: ok\n---\n").is_err()); // no description
        assert!(parse_skill_frontmatter("---\ndescription: x\n---\n").is_err());
        // no name
    }

    #[test]
    fn rejects_unknown_field_and_missing_fence() {
        assert!(parse_skill_frontmatter("---\nname: ok\ndescription: y\nbogus: z\n---\n").is_err());
        assert!(parse_skill_frontmatter("name: ok\ndescription: y\n").is_err());
        assert!(parse_skill_frontmatter("---\nname: ok\ndescription: y\n").is_err());
        // no close
    }

    #[test]
    fn enforces_length_bounds() {
        let long_name = "a".repeat(MAX_NAME_LEN + 1);
        let source = format!("---\nname: {long_name}\ndescription: x\n---\n");
        assert!(parse_skill_frontmatter(&source).is_err());

        let long_desc = "x".repeat(MAX_DESCRIPTION_LEN + 1);
        let source = format!("---\nname: ok\ndescription: {long_desc}\n---\n");
        assert!(parse_skill_frontmatter(&source).is_err());
    }
}
