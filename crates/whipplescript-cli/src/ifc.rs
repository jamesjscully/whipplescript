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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use whipplescript_parser::{Diagnostic, IrProgram};

/// Resource confidentiality, projected from the governance envelope.
pub struct Envelope {
    confidential: BTreeSet<String>,
    governed: BTreeSet<String>,
}

impl Envelope {
    /// Parse the JSON envelope. Shape:
    /// `{ "resources": { "<name>": { "confidential": true|false }, ... } }`.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|err| format!("invalid IFC envelope: {err}"))?;
        let mut confidential = BTreeSet::new();
        let mut governed = BTreeSet::new();
        if let Some(map) = value.get("resources").and_then(|res| res.as_object()) {
            for (name, label) in map {
                governed.insert(name.clone());
                if label
                    .get("confidential")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    confidential.insert(name.clone());
                }
            }
        }
        Ok(Self {
            confidential,
            governed,
        })
    }

    /// Parse the readable governance DSL (DR-0028) into the envelope. v0 grammar,
    /// one statement per line (`#` starts a comment):
    ///   `grant <kind> <handle> -> <resource-id> <label>`
    /// where `<label>` is a `readable by <roles>` clause (confidential) or an
    /// `audience { … }` / `public` clause (un-cleared sink). `party` / `delegate`
    /// statements are accepted and ignored in this binary v0 (they carry the
    /// party-relative content of a later slice).
    pub fn from_dsl(text: &str) -> Result<Self, String> {
        let mut confidential = BTreeSet::new();
        let mut governed = BTreeSet::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            match tokens.first().copied() {
                Some("party") | Some("delegate") => continue,
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
            let label = tokens[arrow + 2..].join(" ");
            governed.insert(handle.clone());
            if label.contains("readable by") || label == "confidential" {
                confidential.insert(handle);
            }
        }
        Ok(Self {
            confidential,
            governed,
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

    /// The canonical signed-artifact JSON: every governed resource with its
    /// confidentiality, in sorted order (so the hash is deterministic). This is
    /// what the governance agent signs (DR-0028).
    pub fn to_canonical_json(&self) -> String {
        let mut resources = serde_json::Map::new();
        for name in &self.governed {
            resources.insert(
                name.clone(),
                serde_json::json!({ "confidential": self.confidential.contains(name) }),
            );
        }
        serde_json::json!({ "resources": resources }).to_string()
    }

    fn is_confidential(&self, resource: &str) -> bool {
        self.confidential.contains(resource)
    }

    /// Confidential data may flow only to a confidential-cleared sink; any other
    /// resource — governed-public OR ungoverned — is an un-cleared sink. This is
    /// the fail-closed sticky boundary (DR-0027 I-IFC6): confidential data cannot
    /// escape into the ungoverned region, so a write target must be confidential
    /// to receive it.
    fn is_uncleared_sink(&self, resource: &str) -> bool {
        !self.confidential.contains(resource)
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

/// The turn-level join-box check: a turn that reads a confidential resource and
/// egresses to an un-cleared one is flagged.
pub fn check_with_envelope(ir: &IrProgram, envelope: &Envelope) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            let mut confidential_read: Option<&str> = None;
            let mut uncleared_sink: Option<&str> = None;
            for grant in &effect.access_grants {
                let resource = grant.resource.as_str();
                for op in &grant.operations {
                    if confidential_read.is_none()
                        && is_read_op(&op.operation)
                        && envelope.is_confidential(resource)
                    {
                        confidential_read = Some(resource);
                    }
                    if uncleared_sink.is_none()
                        && is_egress_op(&op.operation)
                        && envelope.is_uncleared_sink(resource)
                    {
                        uncleared_sink = Some(resource);
                    }
                }
            }
            if let (Some(src), Some(sink)) = (confidential_read, uncleared_sink) {
                diagnostics.push(Diagnostic {
                    span: effect.span,
                    message: format!(
                        "information-flow violation in rule `{rule}`: this turn may read \
                         confidential `{src}` and write un-cleared `{sink}`, so the confidential \
                         data could flow out",
                        rule = rule.name,
                    ),
                    suggestion: Some(format!(
                        "separate the contexts — read `{src}` in a distinct turn and pass only a \
                         bounded result; or declassify the value before writing `{sink}`"
                    )),
                    related: Vec::new(),
                });
            }
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
}

pub fn governance_report(ir: &IrProgram, envelope: &Envelope) -> GovernanceReport {
    let mut protected: Vec<String> = envelope.confidential.iter().cloned().collect();
    protected.sort();
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
        // ledger confidential, outbox governed-but-public — same as the JSON envelope.
        assert!(from_dsl.is_confidential("ledger"));
        assert!(!from_dsl.is_confidential("outbox"));
        assert!(from_dsl.is_uncleared_sink("outbox"));
        assert!(!from_dsl.is_uncleared_sink("ledger"));

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
