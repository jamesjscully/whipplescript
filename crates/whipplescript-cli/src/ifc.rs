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
use std::path::PathBuf;

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

/// An envelope that has crossed the trust boundary: a consumer may safely derive a
/// trusted decision (enforce, or vouch in the guarantee report) from it. It is
/// constructed ONLY by `VerifiedEnvelope::load_from_env`, which verifies a signed
/// policy's attestation first; there is no public path from a signed artifact to a
/// usable envelope that skips verification. So a new consumer cannot reintroduce the
/// report-vs-check bug — it has nothing un-verified to consume. This is the Rust
/// realization of `models/lean/Whipple/Boundary.lean`.
pub struct VerifiedEnvelope {
    envelope: Envelope,
}

/// The outcome of crossing the trust boundary.
pub enum EnvelopeStatus {
    /// No envelope configured: ungoverned dev mode (the gradual model).
    Ungoverned,
    /// Present and authentic — an unsigned dev policy, or signed + verified.
    Verified(VerifiedEnvelope),
    /// Present but its attestation failed: a tampered or re-edited signed policy.
    Rejected(String),
}

impl VerifiedEnvelope {
    /// THE trust boundary. Reads the env-configured policy and, if it carries an
    /// attestation, verifies it before yielding a usable envelope. Every consumer
    /// goes through here, so verification is enforced once, for all of them.
    pub fn load_from_env() -> EnvelopeStatus {
        let Some(path) = envelope_path_from_env() else {
            return EnvelopeStatus::Ungoverned;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return EnvelopeStatus::Ungoverned;
        };
        Self::from_text(&text)
    }

    /// Cross the boundary from envelope text (the testable core of `load_from_env`).
    /// A signed policy (one carrying an `attestation`) must verify before it can be
    /// wrapped; a tampered one is `Rejected` and never becomes a `VerifiedEnvelope`.
    fn from_text(text: &str) -> EnvelopeStatus {
        if text.contains("\"attestation\"") {
            if let Err(message) = crate::gov::SignedEnvelope::verify(text) {
                return EnvelopeStatus::Rejected(message);
            }
        }
        let parsed = if text.trim_start().starts_with('{') {
            Envelope::from_json(text)
        } else {
            Envelope::from_dsl(text)
        };
        match parsed {
            Ok(envelope) => EnvelopeStatus::Verified(VerifiedEnvelope { envelope }),
            // A malformed envelope is the governance compiler's error to report, not
            // the checker's; treat as ungoverned (the prior behavior).
            Err(_) => EnvelopeStatus::Ungoverned,
        }
    }

    /// The verified envelope. Crate-internal: only the gated consumers in this
    /// module read it, and only once they hold a `VerifiedEnvelope`.
    fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// Wrap a raw envelope as verified — TESTS ONLY (unit tests exercise the checker
    /// algebra directly, without the signing boundary), mirroring
    /// `gov::SignedEnvelope::sign_for_test`.
    #[cfg(test)]
    pub(crate) fn for_test(envelope: Envelope) -> Self {
        Self { envelope }
    }
}

/// The rendered guarantee report for a `whip check` run, if a governance envelope
/// is configured; `None` in dev mode. Routes through the trust boundary: a tampered
/// signed policy yields a refusal note, never a guarantee computed from tampered
/// labels (the report must not vouch for content it cannot attest).
pub fn report_for_check(ir: &IrProgram) -> Option<String> {
    match VerifiedEnvelope::load_from_env() {
        EnvelopeStatus::Ungoverned => None,
        EnvelopeStatus::Rejected(message) => Some(format!(
            "information-flow guarantee report\n  REFUSED: {message}\n"
        )),
        EnvelopeStatus::Verified(verified) => Some(governance_report(ir, &verified).render()),
    }
}

/// Run the IFC check if a governance envelope is configured; otherwise no
/// constraints apply (dev mode) and this returns no diagnostics. Routes through the
/// trust boundary: a signed policy is verified first, and the whip agent refuses to
/// enforce a tampered one.
pub fn check_ifc_program(ir: &IrProgram) -> Vec<Diagnostic> {
    match VerifiedEnvelope::load_from_env() {
        EnvelopeStatus::Ungoverned => Vec::new(),
        EnvelopeStatus::Rejected(message) => vec![Diagnostic {
            span: whipplescript_parser::SourceSpan { start: 0, end: 0 },
            message: format!("governance envelope rejected: {message}"),
            suggestion: Some(
                "re-sign the envelope with `whip gov sign` after editing it".to_owned(),
            ),
            related: Vec::new(),
        }],
        EnvelopeStatus::Verified(verified) => check_with_envelope(ir, &verified),
    }
}

/// The turn-level join-box check: for a turn that reads resource `src` and writes
/// resource `sink`, flag the pair when data from `src` may leak to a reader of
/// `sink` not cleared for `src` (party-relative, via the acts-for closure).
pub fn check_with_envelope(ir: &IrProgram, verified: &VerifiedEnvelope) -> Vec<Diagnostic> {
    let envelope = verified.envelope();
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
                    // `send via <channel>` lowers to a capability call carrying the
                    // channel as its resource; it is an egress sink.
                    IrEffectKind::CapabilityCall => {
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
            // Provider egress (DR-0027 provider-as-principal): a turn ships its
            // context to the agent's model provider, so a read-confidential turn
            // whose provider is not cleared leaks to the model.
            if effect.kind == IrEffectKind::AgentTell {
                if let Some(provider) = effect
                    .agent
                    .as_deref()
                    .and_then(|name| ir.agents.iter().find(|a| a.name == name))
                    .and_then(|a| a.provider.as_deref())
                {
                    for grant in &effect.access_grants {
                        let resource = grant.resource.as_str();
                        let reads_resource =
                            grant.operations.iter().any(|op| is_read_op(&op.operation));
                        if reads_resource && envelope.leaks(resource, provider) {
                            diagnostics.push(Diagnostic {
                                span: effect.span,
                                message: format!(
                                    "provider-egress violation in rule `{rule}`: a turn reads \
                                     `{resource}` (readable by {rr}) but its provider `{provider}` \
                                     (clearance {pr}) is not cleared, so the turn's context egresses \
                                     to an uncleared model",
                                    rule = rule.name,
                                    rr = envelope.reader_authority(resource),
                                    pr = envelope.reader_authority(provider),
                                ),
                                suggestion: Some(format!(
                                    "bind the agent to a provider cleared for `{resource}`, or \
                                     declassify before the turn"
                                )),
                                related: Vec::new(),
                            });
                            break;
                        }
                    }
                }
            }
        }
        // `record <Fact>` writes the durable fact-base, which other rules and the
        // DR-0026 session-event stream observe — a governed egress sink (the
        // recordSink of infoflow-composition, H2). Sink id `fact:<schema>`;
        // unlabeled defaults to public (fail-closed), so confidential data cannot
        // silently leave a governed flow via a recorded fact, and untrusted data
        // cannot drive a high-integrity fact governance has labelled. `fact_writes`
        // carries the recorded schemas as `schema:<Name>`.
        let record_sinks: Vec<String> = rule
            .metadata
            .fact_writes
            .iter()
            .map(|write| format!("fact:{}", write.strip_prefix("schema:").unwrap_or(write)))
            .collect();
        // Inbound `when message from <channel>` delivers attacker-controllable
        // content: the channel is a low-integrity READ source (and public
        // confidentiality), so untrusted inbound data driving a more-trusted sink is
        // caught as an injection (H3). The IR pattern is `message from <channel>`.
        let mut message_reads: Vec<&str> = Vec::new();
        for when in &rule.whens {
            if let Some(rest) = when.pattern.trim_start().strip_prefix("message from ") {
                if let Some(channel) = rest.split_whitespace().next() {
                    message_reads.push(channel);
                }
            }
        }
        let report_span = span.unwrap_or(whipplescript_parser::SourceSpan { start: 0, end: 0 });
        let mut leak: Option<(&str, &str)> = None;
        let mut inject: Option<(&str, &str)> = None;
        for src in reads.iter().copied().chain(message_reads.iter().copied()) {
            for sink in writes
                .iter()
                .copied()
                .chain(record_sinks.iter().map(String::as_str))
            {
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
    /// The audited trusted surface: each crossing, tagged by axis —
    /// `declassify <resource> -> <role>` and `endorse <resource> -> <role>`.
    pub trusted_surface: Vec<String>,
}

pub fn governance_report(ir: &IrProgram, verified: &VerifiedEnvelope) -> GovernanceReport {
    let envelope = verified.envelope();
    // protected = governed resources whose reader authority is not public.
    let mut protected: Vec<String> = envelope.readers.keys().cloned().collect();
    protected.sort();
    // The audited trusted surface is BOTH axes' crossings: declassify (lowers
    // confidentiality) and endorse (raises integrity). Endorse is at least as
    // risky -- it lets less-trusted data drive a more-trusted sink -- so it must be
    // reviewable too (H4). Each is tagged with its axis.
    let mut trusted_surface: Vec<String> = envelope
        .declassify
        .iter()
        .map(|(resource, role)| format!("declassify {resource} -> {role}"))
        .chain(
            envelope
                .endorse
                .iter()
                .map(|(resource, role)| format!("endorse {resource} -> {role}")),
        )
        .collect();
    trusted_surface.sort();
    let violations = check_with_envelope(ir, verified).len();
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
            out.push_str("  trusted surface (declassify + endorse grants): none\n");
        } else {
            out.push_str("  trusted surface (audited declassify/endorse crossings to review):\n");
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
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
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
    fn allows_turn_reading_confidential_only_when_provider_cleared() {
        let ir = ir_with_grants(READ_LEDGER);
        // the agent's `fixture` provider is cleared for confidential data, so a
        // read-only turn with no egress is fine.
        let envelope = Envelope::from_json(
            r#"{ "resources": {
                "ledger": { "confidential": true },
                "fixture": { "reader": "confidential" }
            } }"#,
        )
        .expect("valid envelope");
        assert!(check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope)).is_empty());
    }

    #[test]
    fn flags_provider_egress_to_uncleared_provider() {
        let ir = ir_with_grants(READ_LEDGER);
        // ledger confidential, fixture provider unlabeled (public clearance): the
        // turn's context egresses to an uncleared model.
        let envelope =
            Envelope::from_json(r#"{ "resources": { "ledger": { "confidential": true } } }"#)
                .expect("valid envelope");
        assert!(
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope))
                .iter()
                .any(|d| d.message.contains("provider-egress violation")),
            "reading confidential data with an uncleared provider should be flagged"
        );
    }

    #[test]
    fn ungoverned_resources_are_unconstrained() {
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        // empty envelope: nothing is governed, so the gradual model imposes nothing.
        let envelope = Envelope::from_json(r#"{ "resources": {} }"#).expect("valid envelope");
        assert!(check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope)).is_empty());
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
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(from_dsl))
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
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
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
    fn rule_body_send_via_channel_is_an_egress() {
        // read a confidential store and `send via` a (public) channel -> leak.
        let program = r##"@service
workflow IfcSend

output result R
class R { ok bool }
class Ticket { id string  status "open" }

file store ledger { root "./ledger"  allow read ["**"] }
channel reply { provider slack  destination "#out" }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from ledger at "data.txt" as loaded
  send via reply { text "x" } as sent
  complete result { ok true }
}
"##;
        let ir = compile_program(program).ir.expect("compiles");
        let envelope =
            Envelope::from_json(r#"{ "resources": { "ledger": { "confidential": true } } }"#)
                .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("ledger")
                    && d.message.contains("reply")),
            "send via a public channel should be flagged as egress, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
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
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
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

    #[test]
    fn inbound_message_is_a_low_integrity_source() {
        // a rule triggered by `when message from <channel>` reads attacker-
        // controllable content; letting it drive a high-integrity sink (here a
        // file write to an Operator-integrity store) is an injection (H3).
        let program = r##"@service
workflow IfcInbound

output result R
class R { ok bool }

channel intake { provider slack  destination "#in" }
file store ledger { root "./ledger"  allow write ["**"] }

rule ingest
  when message from intake as msg
=> {
  write text to ledger at "notes.txt" {
    body "{{ msg.text }}"
    mode append
  } as noted
  after noted succeeds {
    complete result { ok true }
  }
}
"##;
        let ir = compile_program(program).ir.expect("compiles");
        // intake is untrusted (public integrity); ledger requires Operator integrity.
        let envelope = Envelope::from_dsl(
            "grant channel intake -> imap:in from public\n\
             grant file_store ledger -> file:/srv/ledger.db from Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("integrity violation")
                    && d.message.contains("intake")
                    && d.message.contains("ledger")),
            "inbound message driving a trusted sink should be an injection, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn record_to_fact_base_is_a_governed_sink() {
        // reading a confidential store and `record`ing a fact derived from it leaks
        // to the fact-base, which other rules and the DR-0026 stream observe (H2).
        // `fact:<schema>` defaults to public, so it is caught fail-closed.
        let program = r#"@service
workflow IfcRecord

output result R
class R { ok bool }
class Note { id string }
class Ticket { id string  status "open" }

file store ledger { root "./ledger"  allow read ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from ledger at "data.txt" as loaded
  after loaded succeeds as file {
    record Note { id file.content }
  }
}

rule finish
  when Note as note
=> {
  complete result { ok true }
}
"#;
        let ir = compile_program(program).ir.expect("compiles");
        let envelope =
            Envelope::from_json(r#"{ "resources": { "ledger": { "confidential": true } } }"#)
                .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("ledger")
                    && d.message.contains("fact:Note")),
            "record of confidential-derived fact should leak to the fact-base, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn trusted_surface_audits_both_declassify_and_endorse() {
        // both crossings must be reviewable: declassify (lowers confidentiality)
        // and endorse (raises integrity). The report tags each by axis (H4).
        let envelope = Envelope::from_dsl(
            "grant file_store ledger -> file:/srv/ledger.db readable by Operator from Operator\n\
             grant channel intake -> imap:in from public\n\
             grant declassify ledger to Requester\n\
             grant endorse intake to Operator\n",
        )
        .expect("valid");
        let ir = ir_with_grants(READ_LEDGER);
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            report
                .trusted_surface
                .contains(&"declassify ledger -> Requester".to_owned()),
            "declassify crossing should be audited: {:?}",
            report.trusted_surface
        );
        assert!(
            report
                .trusted_surface
                .contains(&"endorse intake -> Operator".to_owned()),
            "endorse crossing should be audited too (H4): {:?}",
            report.trusted_surface
        );
    }

    #[test]
    fn report_refuses_a_tampered_signed_envelope() {
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        let config = "grant file_store ledger -> file:/srv/ledger.db readable by Operator\n";
        let signed = crate::gov::SignedEnvelope::sign_for_test(config, "admin");
        let json = signed.to_json();
        // a valid signed envelope crosses the boundary and renders a guarantee...
        match VerifiedEnvelope::from_text(&json) {
            EnvelopeStatus::Verified(verified) => {
                let ok = governance_report(&ir, &verified).render();
                assert!(ok.contains("protected resources"));
            }
            _ => panic!("genuine signed envelope should verify"),
        }
        // ...but tampering with the labels makes the boundary REJECT it, so neither
        // the checker nor the report can vouch for content they cannot attest.
        let tampered = json.replace("\"reader\":\"Operator\"", "\"reader\":\"public\"");
        assert_ne!(tampered, json, "test should actually modify the content");
        match VerifiedEnvelope::from_text(&tampered) {
            EnvelopeStatus::Rejected(_) => {}
            _ => panic!("tampered signed envelope must be rejected"),
        }
    }
}
