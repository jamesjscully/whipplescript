//! Information-flow control checking (DR-0027 / DR-0028) — first vertical slice.
//!
//! A governance envelope (JSON; the signed-artifact form that the DR-0028
//! governance DSL will compile to) labels real resources by confidentiality. This
//! slice enforces the **turn-level join box** (DR-0027 I-IFC2): an agent turn
//! granted a READ on a confidential resource and a WRITE/egress on an un-cleared
//! resource could carry the confidential data out, so it is rejected — unless the
//! contexts are separated or the value is declassified.
//!
//! Scope of this slice: binary confidentiality, turn-grant granularity. The
//! party-relative labels (the gate-green Maude models) and the source crossings
//! (`endorsed` / `declassify`) arrive in later slices.
//!
//! Discovery follows the gradual model (DR-0027 I-IFC6): `WHIPPLESCRIPT_IFC_ENVELOPE`
//! points at the envelope; unset = ungoverned dev mode (a plain whip making no IFC
//! claim).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use whipplescript_parser::{Diagnostic, IrEffectKind, IrProgram};

/// The bottom reader-authority: data readable by `public` is readable by anyone,
/// and `public` itself holds no authority above itself.
const PUBLIC: &str = "public";

/// The party-relative confidentiality projection of the governance envelope
/// (DR-0027 I-IFC1): each governed resource has a **reader authority** (a role);
/// the secret is readable by any party that acts-for that role. The delegation
/// context is the acts-for edge set, closed reflexive-transitively by `can_act`.
pub struct Envelope {
    /// resource handle -> reader-authority role (absent = `public`, the bottom).
    readers: BTreeMap<String, String>,
    governed: BTreeSet<String>,
    /// acts-for edges `(p, q)`: `p` acts-for `q` (has at least `q`'s authority).
    deleg: Vec<(String, String)>,
    /// declassify grants `(resource, role)`: `resource` may be released to any
    /// party that acts-for `role`. These are the audited trusted-surface holes.
    declassify: Vec<(String, String)>,
    /// integrity (writer/vouching) authority per resource (absent = `public`, the
    /// untrusted bottom). A control sink requiring integrity `r` accepts data only
    /// from a source whose integrity acts-for `r` (DR-0027 I-IFC1, integrity axis).
    integrity: BTreeMap<String, String>,
    /// endorse grants `(resource, role)`: `resource`'s data may be raised to `role`
    /// integrity — the audited integrity-axis crossing.
    endorse: Vec<(String, String)>,
}

impl Envelope {
    /// Parse the JSON envelope. Resources carry a `reader` role (or, for
    /// back-compat, a `confidential` bool: true = reader `confidential`, false =
    /// public). Optional `delegations` is an array of `[p, q]` acts-for pairs.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|err| format!("invalid IFC envelope: {err}"))?;
        let mut readers = BTreeMap::new();
        let mut governed = BTreeSet::new();
        let mut deleg = Vec::new();
        let mut declassify = Vec::new();
        let mut integrity = BTreeMap::new();
        let mut endorse = Vec::new();
        if let Some(map) = value.get("resources").and_then(|res| res.as_object()) {
            for (name, label) in map {
                governed.insert(name.clone());
                if let Some(reader) = label.get("reader").and_then(serde_json::Value::as_str) {
                    if reader != PUBLIC {
                        readers.insert(name.clone(), reader.to_owned());
                    }
                } else if label
                    .get("confidential")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    readers.insert(name.clone(), "confidential".to_owned());
                }
                if let Some(writer) = label.get("writer").and_then(serde_json::Value::as_str) {
                    if writer != PUBLIC {
                        integrity.insert(name.clone(), writer.to_owned());
                    }
                }
            }
        }
        if let Some(pairs) = value.get("endorsements").and_then(|d| d.as_array()) {
            for pair in pairs {
                if let Some(items) = pair.as_array() {
                    if let (Some(res), Some(role)) = (
                        items.first().and_then(serde_json::Value::as_str),
                        items.get(1).and_then(serde_json::Value::as_str),
                    ) {
                        endorse.push((res.to_owned(), role.to_owned()));
                    }
                }
            }
        }
        if let Some(pairs) = value.get("delegations").and_then(|d| d.as_array()) {
            for pair in pairs {
                if let Some(items) = pair.as_array() {
                    if let (Some(left), Some(right)) = (
                        items.first().and_then(serde_json::Value::as_str),
                        items.get(1).and_then(serde_json::Value::as_str),
                    ) {
                        deleg.push((left.to_owned(), right.to_owned()));
                    }
                }
            }
        }
        if let Some(pairs) = value.get("declassifications").and_then(|d| d.as_array()) {
            for pair in pairs {
                if let Some(items) = pair.as_array() {
                    if let (Some(res), Some(role)) = (
                        items.first().and_then(serde_json::Value::as_str),
                        items.get(1).and_then(serde_json::Value::as_str),
                    ) {
                        declassify.push((res.to_owned(), role.to_owned()));
                    }
                }
            }
        }
        Ok(Self {
            readers,
            governed,
            deleg,
            declassify,
            integrity,
            endorse,
        })
    }

    /// Parse the readable governance DSL (DR-0028), one statement per line:
    ///   `grant <kind> <handle> -> <resource-id> readable by <Role>`  (reader = Role)
    ///   `grant <kind> <handle> -> <resource-id> public | audience { … }` (public)
    ///   `delegate <P> acts-for <Q> [for <axis>]`  (acts-for edge P -> Q)
    /// `party` lines are accepted and ignored (the runtime binds parties to roles).
    pub fn from_dsl(text: &str) -> Result<Self, String> {
        let mut readers = BTreeMap::new();
        let mut governed = BTreeSet::new();
        let mut deleg = Vec::new();
        let mut declassify = Vec::new();
        let mut integrity = BTreeMap::new();
        let mut endorse = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            // `grant declassify|endorse <resource> to <role>` — the audited crossings.
            if tokens.first().copied() == Some("grant")
                && matches!(tokens.get(1).copied(), Some("declassify") | Some("endorse"))
            {
                let kind = tokens[1];
                let Some(to) = tokens.iter().position(|tok| *tok == "to") else {
                    return Err(format!(
                        "line {}: {kind} grant needs `to <role>`",
                        index + 1
                    ));
                };
                if to < 3 || to + 1 >= tokens.len() {
                    return Err(format!(
                        "line {}: {kind} grant needs `grant {kind} <resource> to <role>`",
                        index + 1
                    ));
                }
                let pair = (tokens[2].to_owned(), tokens[to + 1].to_owned());
                if kind == "declassify" {
                    declassify.push(pair);
                } else {
                    endorse.push(pair);
                }
                continue;
            }
            match tokens.first().copied() {
                Some("party") => continue,
                Some("delegate") => {
                    let Some(pos) = tokens.iter().position(|tok| *tok == "acts-for") else {
                        return Err(format!("line {}: delegate needs `acts-for`", index + 1));
                    };
                    if pos < 1 || pos + 1 >= tokens.len() {
                        return Err(format!(
                            "line {}: delegate needs `delegate <P> acts-for <Q>`",
                            index + 1
                        ));
                    }
                    deleg.push((tokens[pos - 1].to_owned(), tokens[pos + 1].to_owned()));
                    continue;
                }
                Some("grant") => {}
                _ => {
                    return Err(format!(
                        "line {}: unrecognized governance statement",
                        index + 1
                    ));
                }
            }
            let arrow = tokens.iter().position(|tok| *tok == "->");
            let Some(arrow) = arrow.filter(|pos| *pos >= 3 && *pos + 1 < tokens.len()) else {
                return Err(format!(
                    "line {}: grant needs `grant <kind> <handle> -> <resource-id> <label>`",
                    index + 1
                ));
            };
            let handle = tokens[arrow - 1].to_owned();
            governed.insert(handle.clone());
            let label = &tokens[arrow + 2..];
            // `readable by <Role>` sets a non-public reader authority.
            if let Some(by) = label.iter().position(|tok| *tok == "by") {
                if let Some(role) = label.get(by + 1) {
                    if *role != PUBLIC {
                        readers.insert(handle.clone(), (*role).to_owned());
                    }
                }
            }
            // `from <Role>` sets the integrity (vouching) authority.
            if let Some(from) = label.iter().position(|tok| *tok == "from") {
                if let Some(role) = label.get(from + 1) {
                    if *role != PUBLIC {
                        integrity.insert(handle, (*role).to_owned());
                    }
                }
            }
        }
        Ok(Self {
            readers,
            governed,
            deleg,
            declassify,
            integrity,
            endorse,
        })
    }

    /// Load a governance envelope, auto-detecting JSON (the signed artifact) from
    /// the readable DSL by the first non-whitespace character.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("cannot read IFC envelope {}: {err}", path.display()))?;
        if text.trim_start().starts_with('{') {
            Self::from_json(&text)
        } else {
            Self::from_dsl(&text)
        }
    }

    /// The canonical signed-artifact JSON: every governed resource with its reader
    /// authority, plus the delegation edges, all sorted (deterministic hash).
    pub fn to_canonical_json(&self) -> String {
        let mut resources = serde_json::Map::new();
        for name in &self.governed {
            resources.insert(
                name.clone(),
                serde_json::json!({
                    "reader": self.reader_authority(name),
                    "writer": self.integrity_authority(name),
                }),
            );
        }
        let mut endorsed: Vec<(String, String)> = self.endorse.clone();
        endorsed.sort();
        let endorsements: Vec<serde_json::Value> = endorsed
            .iter()
            .map(|(res, role)| serde_json::json!([res, role]))
            .collect();
        let mut edges: Vec<(String, String)> = self.deleg.clone();
        edges.sort();
        let delegations: Vec<serde_json::Value> = edges
            .iter()
            .map(|(left, right)| serde_json::json!([left, right]))
            .collect();
        let mut declass: Vec<(String, String)> = self.declassify.clone();
        declass.sort();
        let declassifications: Vec<serde_json::Value> = declass
            .iter()
            .map(|(res, role)| serde_json::json!([res, role]))
            .collect();
        serde_json::json!({
            "resources": resources,
            "delegations": delegations,
            "declassifications": declassifications,
            "endorsements": endorsements,
        })
        .to_string()
    }

    /// The reader authority of a resource; `public` (the bottom) if unlabeled.
    fn reader_authority(&self, resource: &str) -> &str {
        self.readers
            .get(resource)
            .map(String::as_str)
            .unwrap_or(PUBLIC)
    }

    /// `p` acts-for `q`: reflexive-transitive over the delegation edges, with
    /// `public` as the universal bottom (everyone acts-for `public`; `public`
    /// acts-for nothing but itself). Cycle-safe via a visited set.
    fn can_act(&self, p: &str, q: &str) -> bool {
        if q == PUBLIC || p == q {
            return true;
        }
        if p == PUBLIC {
            return false;
        }
        let mut frontier = vec![p.to_owned()];
        let mut visited = BTreeSet::new();
        while let Some(current) = frontier.pop() {
            if current == q {
                return true;
            }
            if !visited.insert(current.clone()) {
                continue;
            }
            for (left, right) in &self.deleg {
                if *left == current {
                    frontier.push(right.clone());
                }
            }
        }
        false
    }

    /// Does data from `source` leak when written to `sink`? Safe iff every party
    /// that can read `sink` can also read `source` — i.e. `sink`'s reader authority
    /// acts-for `source`'s — OR a declassify grant releases `source` to a role the
    /// `sink` reader is cleared for (the audited escape hatch, DR-0027 I-IFC3).
    /// Otherwise some reader of `sink` is not cleared for `source`, and it leaks
    /// (the fail-closed sticky boundary, DR-0027 I-IFC6).
    fn leaks(&self, source: &str, sink: &str) -> bool {
        let sink_reader = self.reader_authority(sink);
        if self.can_act(sink_reader, self.reader_authority(source)) {
            return false;
        }
        for (resource, role) in &self.declassify {
            if resource == source && self.can_act(sink_reader, role) {
                return false;
            }
        }
        true
    }

    /// The integrity (vouching) authority of a resource; `public` (the untrusted
    /// bottom) if unlabeled.
    fn integrity_authority(&self, resource: &str) -> &str {
        self.integrity
            .get(resource)
            .map(String::as_str)
            .unwrap_or(PUBLIC)
    }

    /// Does reading `read` and writing `write` inject? Untrusted data pollutes a
    /// trusted sink: safe iff `read`'s integrity acts-for `write`'s requirement (the
    /// dual of `leaks`), OR an endorse grant raises `read` to a role that meets the
    /// requirement (the audited integrity crossing, DR-0027 I-IFC3).
    fn injects(&self, read: &str, write: &str) -> bool {
        let requirement = self.integrity_authority(write);
        if self.can_act(self.integrity_authority(read), requirement) {
            return false;
        }
        for (resource, role) in &self.endorse {
            if resource == read && self.can_act(role, requirement) {
                return false;
            }
        }
        true
    }
}

fn is_read_op(operation: &str) -> bool {
    matches!(operation, "read" | "recall" | "get" | "list" | "import")
}

fn is_egress_op(operation: &str) -> bool {
    matches!(
        operation,
        "write" | "learn" | "send" | "notify" | "emit" | "export" | "append" | "queue"
    )
}

/// The env-discovered envelope path; `None` = ungoverned dev mode.
pub fn envelope_path_from_env() -> Option<PathBuf> {
    std::env::var_os("WHIPPLESCRIPT_IFC_ENVELOPE").map(PathBuf::from)
}

/// The rendered guarantee report for a `whip check` run, if a governance envelope
/// is configured; `None` in dev mode.
pub fn report_for_check(ir: &IrProgram) -> Option<String> {
    let path = envelope_path_from_env()?;
    let envelope = Envelope::load(&path).ok()?;
    Some(governance_report(ir, &envelope).render())
}

/// Run the IFC check if a governance envelope is configured; otherwise no
/// constraints apply (dev mode) and this returns no diagnostics. A *signed*
/// envelope (one carrying an `attestation`) is verified first: the whip agent
/// refuses to enforce a tampered policy.
pub fn check_ifc_program(ir: &IrProgram) -> Vec<Diagnostic> {
    let Some(path) = envelope_path_from_env() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    if text.contains("\"attestation\"") {
        if let Err(message) = crate::gov::SignedEnvelope::verify(&text) {
            return vec![Diagnostic {
                span: whipplescript_parser::SourceSpan { start: 0, end: 0 },
                message: format!("governance envelope rejected: {message}"),
                suggestion: Some(
                    "re-sign the envelope with `whip gov sign` after editing it".to_owned(),
                ),
                related: Vec::new(),
            }];
        }
    }
    match Envelope::load(&path) {
        Ok(envelope) => check_with_envelope(ir, &envelope),
        // A malformed envelope is the governance compiler's error to report (a
        // later slice), not `whip check`'s; do not block the check on it.
        Err(_) => Vec::new(),
    }
}

/// The turn-level join-box check: for a turn that reads resource `src` and writes
/// resource `sink`, flag the pair when data from `src` may leak to a reader of
/// `sink` not cleared for `src` (party-relative, via the acts-for closure).
pub fn check_with_envelope(ir: &IrProgram, envelope: &Envelope) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in &ir.rules {
        // Collect reads and writes across the whole rule (the rule-level join box):
        // both `with access to` turn grants AND direct file effects in the body.
        let mut reads: Vec<&str> = Vec::new();
        let mut writes: Vec<&str> = Vec::new();
        let mut span = None;
        for effect in &rule.metadata.effects {
            if let Some(resource) = &effect.resource {
                match effect.kind {
                    IrEffectKind::FileRead | IrEffectKind::FileImport => {
                        reads.push(resource.as_str());
                        span.get_or_insert(effect.span);
                    }
                    IrEffectKind::FileWrite | IrEffectKind::FileExport => {
                        writes.push(resource.as_str());
                        span.get_or_insert(effect.span);
                    }
                    _ => {}
                }
            }
            for grant in &effect.access_grants {
                let resource = grant.resource.as_str();
                for op in &grant.operations {
                    if is_read_op(&op.operation) {
                        reads.push(resource);
                        span.get_or_insert(effect.span);
                    }
                    if is_egress_op(&op.operation) {
                        writes.push(resource);
                        span.get_or_insert(effect.span);
                    }
                }
            }
        }
        let report_span = span.unwrap_or(whipplescript_parser::SourceSpan { start: 0, end: 0 });
        let mut leak: Option<(&str, &str)> = None;
        let mut inject: Option<(&str, &str)> = None;
        for &src in &reads {
            for &sink in &writes {
                if leak.is_none() && envelope.leaks(src, sink) {
                    leak = Some((src, sink));
                }
                if inject.is_none() && envelope.injects(src, sink) {
                    inject = Some((src, sink));
                }
            }
        }
        if let Some((src, sink)) = leak {
            diagnostics.push(Diagnostic {
                span: report_span,
                message: format!(
                    "information-flow violation in rule `{rule}`: it may read `{src}` (readable by \
                     {src_reader}) and write `{sink}` (readable by {sink_reader}), so data from \
                     `{src}` could reach a party not cleared for it",
                    rule = rule.name,
                    src_reader = envelope.reader_authority(src),
                    sink_reader = envelope.reader_authority(sink),
                ),
                suggestion: Some(format!(
                    "separate the contexts — read `{src}` in a distinct turn and pass only a \
                     bounded result; or declassify the value before writing `{sink}`"
                )),
                related: Vec::new(),
            });
        }
        if let Some((src, sink)) = inject {
            diagnostics.push(Diagnostic {
                span: report_span,
                message: format!(
                    "integrity violation in rule `{rule}`: it may let untrusted `{src}` (integrity \
                     {src_int}) influence `{sink}` (requires integrity {sink_int}), an injection \
                     into a more-trusted sink",
                    rule = rule.name,
                    src_int = envelope.integrity_authority(src),
                    sink_int = envelope.integrity_authority(sink),
                ),
                suggestion: Some(format!(
                    "endorse `{src}` to the required integrity (a `grant endorse {src} to …`), or \
                     do not let `{src}` influence `{sink}`"
                )),
                related: Vec::new(),
            });
        }
    }
    diagnostics
}

/// The IT-facing guarantee report (`gov compile`, DR-0028): what a governance
/// config protects and the risks it leaves. v0 surfaces the protected resources,
/// the count of IFC violations the config catches in the program, and coverage
/// gaps (resources the program touches that governance does not label).
pub struct GovernanceReport {
    pub protected: Vec<String>,
    pub violations: usize,
    pub coverage_gaps: Vec<String>,
    /// The audited trusted surface: each declassify grant `resource -> role`.
    pub trusted_surface: Vec<String>,
}

pub fn governance_report(ir: &IrProgram, envelope: &Envelope) -> GovernanceReport {
    // protected = governed resources whose reader authority is not public.
    let mut protected: Vec<String> = envelope.readers.keys().cloned().collect();
    protected.sort();
    let mut trusted_surface: Vec<String> = envelope
        .declassify
        .iter()
        .map(|(resource, role)| format!("{resource} -> {role}"))
        .collect();
    trusted_surface.sort();
    let violations = check_with_envelope(ir, envelope).len();
    let mut touched: BTreeSet<String> = BTreeSet::new();
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            for grant in &effect.access_grants {
                touched.insert(grant.resource.clone());
            }
        }
    }
    let coverage_gaps: Vec<String> = touched
        .into_iter()
        .filter(|resource| !envelope.governed.contains(resource))
        .collect();
    GovernanceReport {
        protected,
        violations,
        coverage_gaps,
        trusted_surface,
    }
}

impl GovernanceReport {
    /// Render the report as IT-legible text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("information-flow guarantee report\n");
        if self.protected.is_empty() {
            out.push_str("  protected resources: none (no confidentiality declared)\n");
        } else {
            out.push_str("  protected resources (confidential):\n");
            for resource in &self.protected {
                out.push_str(&format!(
                    "    - {resource}: may not flow to an un-cleared sink without a declassify\n"
                ));
            }
        }
        out.push_str(&format!(
            "  violations caught in this program: {}\n",
            self.violations
        ));
        if self.coverage_gaps.is_empty() {
            out.push_str("  coverage gaps: none\n");
        } else {
            out.push_str(
                "  coverage gaps (resources the program touches but governance does not label):\n",
            );
            for resource in &self.coverage_gaps {
                out.push_str(&format!("    - {resource}\n"));
            }
        }
        if self.trusted_surface.is_empty() {
            out.push_str("  trusted surface (declassify grants): none\n");
        } else {
            out.push_str("  trusted surface (audited declassify grants to review):\n");
            for crossing in &self.trusted_surface {
                out.push_str(&format!("    - {crossing}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whipplescript_parser::compile_program;

    const ENVELOPE: &str = r#"{ "resources": {
        "ledger": { "confidential": true },
        "outbox": { "confidential": false }
    } }"#;

    /// Build a whip whose single turn carries the given `with access to` grant
    /// blocks, declaring both a `ledger` (read) and `outbox` (write) file store.
    fn ir_with_grants(grants: &str) -> IrProgram {
        let program = format!(
            r#"@service
workflow IfcTest

output result R
class R {{ ok bool }}
class Ticket {{ id string  status "open" }}

agent coder {{ provider fixture  profile "repo-writer"  capacity 1 }}

file store ledger {{ root "./ledger"  allow read ["**"] }}
file store outbox {{ root "./outbox"  allow write ["**"] }}

table seed as Ticket [ {{ id "T1"  status "open" }} ]

rule work
  when Ticket as ticket where ticket.status == "open"
  when coder is available
=> {{
  tell coder as turn
{grants}  "go"

  after turn succeeds as outcome {{
    complete result {{ ok true }}
  }}
}}
"#
        );
        let compiled = compile_program(&program);
        compiled.ir.unwrap_or_else(|| {
            panic!(
                "fixture should compile, diagnostics: {:?}",
                compiled
                    .diagnostics
                    .iter()
                    .map(|d| &d.message)
                    .collect::<Vec<_>>()
            )
        })
    }

    const READ_LEDGER: &str = "    with access to ledger {\n      read [\"**\"]\n    }\n";
    const WRITE_OUTBOX: &str = "    with access to outbox {\n      write [\"**\"]\n    }\n";

    #[test]
    fn flags_turn_reading_confidential_and_writing_uncleared() {
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        let envelope = Envelope::from_json(ENVELOPE).expect("valid envelope");
        let diagnostics = check_with_envelope(&ir, &envelope);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("ledger")
                    && d.message.contains("outbox")),
            "expected an IFC violation, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn allows_turn_reading_confidential_only() {
        let ir = ir_with_grants(READ_LEDGER);
        let envelope = Envelope::from_json(ENVELOPE).expect("valid envelope");
        assert!(check_with_envelope(&ir, &envelope).is_empty());
    }

    #[test]
    fn ungoverned_resources_are_unconstrained() {
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        // empty envelope: nothing is governed, so the gradual model imposes nothing.
        let envelope = Envelope::from_json(r#"{ "resources": {} }"#).expect("valid envelope");
        assert!(check_with_envelope(&ir, &envelope).is_empty());
    }

    const DSL: &str = "\
# governance for the IFC test\n\
grant file_store ledger -> file:/srv/ledger.db readable by Operator\n\
grant file_store outbox -> file:/srv/outbox audience { Requester }\n\
party bob@acme.com : Requester\n";

    #[test]
    fn dsl_parses_to_the_same_labels_as_json() {
        let from_dsl = Envelope::from_dsl(DSL).expect("valid DSL");
        // ledger has reader authority Operator; outbox is public.
        assert_eq!(from_dsl.reader_authority("ledger"), "Operator");
        assert_eq!(from_dsl.reader_authority("outbox"), "public");
        // ledger (Operator) -> outbox (public) leaks; the reverse does not.
        assert!(from_dsl.leaks("ledger", "outbox"));
        assert!(!from_dsl.leaks("outbox", "ledger"));

        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        assert!(
            check_with_envelope(&ir, &from_dsl)
                .iter()
                .any(|d| d.message.contains("information-flow violation")),
            "DSL-derived envelope should reject the bad flow"
        );
    }

    #[test]
    fn dsl_rejects_a_malformed_grant() {
        assert!(Envelope::from_dsl("grant file_store ledger confidential").is_err());
    }

    #[test]
    fn rule_body_file_flow_is_checked() {
        // a rule that directly reads a confidential store and writes a public one
        // in its body (no agent turn) is flagged via the new resource surfacing.
        let program = r#"@service
workflow IfcBody

output result R
class R { ok bool }
class Ticket { id string  status "open" }

file store ledger { root "./ledger"  allow read ["**"] }
file store outbox { root "./outbox"  allow write ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from ledger at "data.txt" as loaded
  write text to outbox at "out.txt" {
    body "x"
    mode replace
  } as written
  complete result { ok true }
}
"#;
        let ir = compile_program(program).ir.expect("compiles");
        let envelope = Envelope::from_json(ENVELOPE).expect("valid");
        let diagnostics = check_with_envelope(&ir, &envelope);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("ledger")
                    && d.message.contains("outbox")),
            "rule-body read->write should be flagged, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn acts_for_delegation_clears_a_flow() {
        // ledger is Operator-readable; auditbox is Auditor-readable. Operator data
        // to an Auditor sink normally leaks...
        let base = "\
grant file_store ledger -> file:/srv/ledger.db readable by Operator\n\
grant file_store auditbox -> file:/srv/auditbox readable by Auditor\n";
        let without = Envelope::from_dsl(base).expect("valid");
        assert!(without.leaks("ledger", "auditbox"));

        // ...but a delegation `Auditor acts-for Operator` clears it: an auditor is
        // cleared for operator data, so the flow is safe.
        let with = Envelope::from_dsl(&format!(
            "{base}delegate Auditor acts-for Operator for confidentiality\n"
        ))
        .expect("valid");
        assert!(!with.leaks("ledger", "auditbox"));
        // the reverse remains a leak — Operator does not act-for Auditor here.
        assert!(with.leaks("auditbox", "ledger"));
    }

    #[test]
    fn integrity_injection_and_endorse() {
        // intake is untrusted (from Requester... here unlabeled = untrusted bottom);
        // ledger requires Operator integrity to write. Letting intake influence
        // ledger is an injection.
        let base = "\
grant channel intake -> imap:in from public\n\
grant file_store ledger -> file:/srv/ledger.db from Operator\n";
        let env = Envelope::from_dsl(base).expect("valid");
        assert!(env.injects("intake", "ledger"));
        // an endorse grant raising intake to Operator clears it.
        let with = Envelope::from_dsl(&format!("{base}grant endorse intake to Operator\n"))
            .expect("valid");
        assert!(!with.injects("intake", "ledger"));
        // trusted -> untrusted sink never injects.
        assert!(!env.injects("ledger", "intake"));
    }

    #[test]
    fn declassify_grant_clears_a_flow() {
        let base = "\
grant file_store ledger -> file:/srv/ledger.db readable by Operator\n\
grant channel reply -> smtp:out readable by Requester\n";
        // ledger (Operator) -> reply (Requester) normally leaks.
        assert!(Envelope::from_dsl(base)
            .expect("valid")
            .leaks("ledger", "reply"));
        // a declassify grant releasing ledger to Requester clears it (audited hatch).
        let with = Envelope::from_dsl(&format!("{base}grant declassify ledger to Requester\n"))
            .expect("valid");
        assert!(!with.leaks("ledger", "reply"));
        // but it does not clear a flow to a sink the released role can't reach.
        let with2 = Envelope::from_dsl(&format!(
            "{base}grant channel pub -> smtp:pub public\ngrant declassify ledger to Requester\n"
        ))
        .expect("valid");
        // reply is Requester-readable so cleared; a public sink is not Requester-... actually
        // public is the bottom so canAct(public, Requester) is false -> still leaks to public.
        assert!(with2.leaks("ledger", "pub"));
    }

    #[test]
    fn report_surfaces_protections_violations_and_coverage_gaps() {
        // envelope governs ledger (confidential) but NOT outbox, which the whip writes.
        let envelope =
            Envelope::from_json(r#"{ "resources": { "ledger": { "confidential": true } } }"#)
                .expect("valid envelope");
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        let report = governance_report(&ir, &envelope);
        assert_eq!(report.protected, vec!["ledger".to_owned()]);
        // ledger (confidential) flows to outbox (not confidential) -> caught by the
        // fail-closed sticky boundary even though outbox is ungoverned.
        assert!(report.violations >= 1);
        // outbox is touched (written) but ungoverned -> also a coverage gap.
        assert!(report.coverage_gaps.contains(&"outbox".to_owned()));
        let text = report.render();
        assert!(text.contains("protected resources"));
        assert!(text.contains("coverage gaps"));
    }
}
