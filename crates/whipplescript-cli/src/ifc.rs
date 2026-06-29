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
    /// handles that name a PRINCIPAL (a provider/model endpoint, a human) rather
    /// than protected data. A principal carries a clearance (so it may be a sink
    /// target), but it is not itself a secret — so it must not be listed as a
    /// "protected resource" in the guarantee report (H5). Keyed by `kind:address`.
    principals: BTreeSet<String>,
    /// whip-facing handle -> canonical `kind:address` resource identity (DR-0027
    /// E5). A governance grant `<kind> <handle> -> <kind:address>` binds the
    /// handle (the script-local name) to the real resource. All labels above are
    /// keyed by the ADDRESS, so two handles bound to the same real resource share
    /// its label and the stable typed identity — not the script name — is what
    /// governance reasons about. A handle with no binding resolves to itself.
    address_of: BTreeMap<String, String>,
    /// runtime identity -> acts-for role (DR-0031, the `party <id> : <Role>` map).
    /// The agent serving a principal acts-for that principal's role, and no further:
    /// the role is the agent's authority ceiling (D3). An identity with no party
    /// entry is the public bottom (fail-closed). Empty = no per-user scoping declared.
    party_of: BTreeMap<String, String>,
}

impl Envelope {
    /// Resolve a whip-facing handle to its canonical `kind:address` identity; a
    /// handle with no governance binding is its own identity.
    fn resolve<'a>(&'a self, handle: &'a str) -> &'a str {
        self.address_of
            .get(handle)
            .map(String::as_str)
            .unwrap_or(handle)
    }

    /// Whether governance declared any party (opted into per-user identity scoping).
    fn has_parties(&self) -> bool {
        !self.party_of.is_empty()
    }

    /// The acts-for role a principal holds; the public bottom if unmapped (an unknown
    /// principal is cleared for nothing, fail-closed).
    fn role_for_principal(&self, principal: &str) -> &str {
        self.party_of
            .get(principal)
            .map(String::as_str)
            .unwrap_or(PUBLIC)
    }
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
        let mut principals = BTreeSet::new();
        let mut address_of = BTreeMap::new();
        let mut party_of = BTreeMap::new();
        // a signed/canonical envelope carries the handle -> address bindings; a
        // hand-written JSON without them treats each resource key as its own address.
        if let Some(map) = value.get("bindings").and_then(|b| b.as_object()) {
            for (handle, address) in map {
                if let Some(address) = address.as_str() {
                    address_of.insert(handle.clone(), address.to_owned());
                }
            }
        }
        // identity -> role parties (DR-0031), round-tripped through the signed artifact.
        if let Some(map) = value.get("parties").and_then(|p| p.as_object()) {
            for (identity, role) in map {
                if let Some(role) = role.as_str() {
                    party_of.insert(identity.clone(), role.to_owned());
                }
            }
        }
        if let Some(map) = value.get("resources").and_then(|res| res.as_object()) {
            for (name, label) in map {
                governed.insert(name.clone());
                if label
                    .get("principal")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    principals.insert(name.clone());
                }
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
            principals,
            address_of,
            party_of,
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
        let mut principals = BTreeSet::new();
        let mut address_of: BTreeMap<String, String> = BTreeMap::new();
        let mut party_of: BTreeMap<String, String> = BTreeMap::new();
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
                // `party <identity> : <Role>` binds a runtime identity to an acts-for
                // role (DR-0031). The identity is whatever the principal seam asserts
                // (an OS user, a launcher-passed id); the role becomes its ceiling.
                Some("party") => {
                    if let Some(colon) = tokens.iter().position(|tok| *tok == ":") {
                        if colon >= 2 {
                            if let Some(role) = tokens.get(colon + 1) {
                                party_of.insert(tokens[1].to_owned(), (*role).to_owned());
                            }
                        }
                    }
                    continue;
                }
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
            // the `<kind:address>` after `->` is the canonical resource identity;
            // bind the handle to it and key all labels by the ADDRESS (E5).
            let address = tokens[arrow + 1].to_owned();
            address_of.insert(handle, address.clone());
            governed.insert(address.clone());
            // a `provider` or `human` grant names a principal, not protected data.
            if matches!(tokens.get(1).copied(), Some("provider") | Some("human")) {
                principals.insert(address.clone());
            }
            let label = &tokens[arrow + 2..];
            // `readable by <Role>` sets a non-public reader authority.
            if let Some(by) = label.iter().position(|tok| *tok == "by") {
                if let Some(role) = label.get(by + 1) {
                    if *role != PUBLIC {
                        readers.insert(address.clone(), (*role).to_owned());
                    }
                }
            }
            // `from <Role>` sets the integrity (vouching) authority.
            if let Some(from) = label.iter().position(|tok| *tok == "from") {
                if let Some(role) = label.get(from + 1) {
                    if *role != PUBLIC {
                        integrity.insert(address, (*role).to_owned());
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
            principals,
            address_of,
            party_of,
        })
    }

    /// The canonical signed-artifact JSON: every governed resource with its reader
    /// authority, plus the delegation edges, all sorted (deterministic hash).
    pub fn to_canonical_json(&self) -> String {
        let mut resources = serde_json::Map::new();
        for name in &self.governed {
            let mut entry = serde_json::json!({
                "reader": self.reader_authority(name),
                "writer": self.integrity_authority(name),
            });
            if self.principals.contains(name) {
                entry["principal"] = serde_json::Value::Bool(true);
            }
            resources.insert(name.clone(), entry);
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
        // handle -> address bindings, so a signed envelope round-trips its identity
        // resolution (E5). Sorted by the BTreeMap for a deterministic hash.
        let bindings: serde_json::Map<String, serde_json::Value> = self
            .address_of
            .iter()
            .map(|(handle, address)| (handle.clone(), serde_json::Value::String(address.clone())))
            .collect();
        let parties: serde_json::Map<String, serde_json::Value> = self
            .party_of
            .iter()
            .map(|(identity, role)| (identity.clone(), serde_json::Value::String(role.clone())))
            .collect();
        serde_json::json!({
            "resources": resources,
            "bindings": bindings,
            "parties": parties,
            "delegations": delegations,
            "declassifications": declassifications,
            "endorsements": endorsements,
        })
        .to_string()
    }

    /// The reader authority of a resource; `public` (the bottom) if unlabeled.
    fn reader_authority(&self, resource: &str) -> &str {
        self.readers
            .get(self.resolve(resource))
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
            if self.resolve(resource) == self.resolve(source) && self.can_act(sink_reader, role) {
                return false;
            }
        }
        true
    }

    /// The integrity (vouching) authority of a resource; `public` (the untrusted
    /// bottom) if unlabeled.
    fn integrity_authority(&self, resource: &str) -> &str {
        self.integrity
            .get(self.resolve(resource))
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
            if self.resolve(resource) == self.resolve(read) && self.can_act(role, requirement) {
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
        EnvelopeStatus::Verified(verified) => {
            let mut diagnostics = check_with_envelope(ir, &verified);
            // Principal ceiling (DR-0031 / D3): if governance declared parties, the
            // agent acts-for the role of the principal the environment asserts, and
            // may not read beyond that clearance. An unknown principal is the public
            // bottom (fail-closed).
            if verified.envelope().has_parties() {
                let role: String = crate::principal::current_principal()
                    .map(|principal| {
                        verified
                            .envelope()
                            .role_for_principal(&principal)
                            .to_owned()
                    })
                    .unwrap_or_else(|| PUBLIC.to_owned());
                diagnostics.extend(check_principal_ceiling(ir, &verified, &role));
            }
            diagnostics
        }
    }
}

/// The consumer-side cross-package check (DR-0029 X1/X8). For each imported `@tool`
/// (its name + declared IFC surface), every surface element must be GOVERNED by the
/// consumer's envelope — an ungoverned element is a door the consumer's governance
/// cannot see, so the import is flagged fail-closed. Only applies under a governed,
/// verified envelope (dev mode imposes nothing).
pub fn check_imported_tool_surfaces(imported: &[(String, Vec<String>)]) -> Vec<Diagnostic> {
    let EnvelopeStatus::Verified(verified) = VerifiedEnvelope::load_from_env() else {
        return Vec::new();
    };
    imported_surface_gaps(imported, &verified)
        .into_iter()
        .map(|(tool, doors)| Diagnostic {
            span: whipplescript_parser::SourceSpan { start: 0, end: 0 },
            message: format!(
                "cross-package information-flow violation: imported tool `{tool}` opens doors the \
                 governance does not cover: {} — the consumer cannot see into the package, so an \
                 ungoverned door is fail-closed (DR-0029 X1/X8)",
                doors.join(", ")
            ),
            suggestion: Some(format!(
                "govern these resources in the envelope (or bind them as resource params), or do \
                 not import `{tool}`"
            )),
            related: Vec::new(),
        })
        .collect()
}

/// Core of the consumer cross-package check: for each imported tool, the surface
/// elements NOT governed by the consumer envelope. Testable without env.
fn imported_surface_gaps<'a>(
    imported: &'a [(String, Vec<String>)],
    verified: &VerifiedEnvelope,
) -> Vec<(&'a str, Vec<&'a str>)> {
    let envelope = verified.envelope();
    let mut gaps = Vec::new();
    for (tool, surface) in imported {
        let ungoverned: Vec<&str> = surface
            .iter()
            .map(String::as_str)
            .filter(|door| !envelope.governed.contains(envelope.resolve(door)))
            .collect();
        if !ungoverned.is_empty() {
            gaps.push((tool.as_str(), ungoverned));
        }
    }
    gaps
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
            // human.ask is a door (E2, one of the five non-obvious doors): the
            // QUESTION egresses the turn's context to a human, so `askHuman` is an
            // egress sink. (The answer is handled where it is consumed — the
            // `when human answered` trigger below — as a low-integrity source.)
            // Resource id `human`; unlabeled defaults to public (fail-closed).
            if effect.kind == IrEffectKind::HumanAsk {
                writes.push("human");
                span.get_or_insert(effect.span);
            }
            // emit/notify publish an event to the durable log, which the DR-0026
            // session-event stream and the telemetry export both observe (E2, the
            // last two of the five doors). Egress sink `stream`; unlabeled defaults
            // to public, so confidential data in an emitted event is caught.
            if matches!(
                effect.kind,
                IrEffectKind::EventEmit | IrEffectKind::EventNotify
            ) {
                writes.push("stream");
                span.get_or_insert(effect.span);
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
            let pattern = when.pattern.trim_start();
            if let Some(rest) = pattern.strip_prefix("message from ") {
                if let Some(channel) = rest.split_whitespace().next() {
                    message_reads.push(channel);
                }
            }
            // `when human answered <X>` consumes the human's answer — untrusted,
            // attacker-influenceable input, so a low-integrity source (E2).
            if pattern.starts_with("human answered") {
                message_reads.push("human");
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

        // NMIF-on-the-selector (DR §5.6 / §7.4): a crossing (`endorsed`/`declassified`)
        // inside a `case <disc> { … }` arm whose discriminant is low-integrity is
        // rejected — the attacker must not steer which declassify/endorse runs. The
        // discriminant is low-integrity when its root binding comes from a
        // low-integrity `when` source (an inbound message / a human answer).
        let low_integrity_bindings: Vec<&str> = rule
            .whens
            .iter()
            .filter_map(|when| {
                let pattern = when.pattern.trim_start();
                if !(pattern.starts_with("message from ") || pattern.starts_with("human answered"))
                {
                    return None;
                }
                let mut tokens = pattern.split_whitespace();
                while let Some(token) = tokens.next() {
                    if token == "as" {
                        return tokens.next();
                    }
                }
                None
            })
            .collect();
        for effect in &rule.metadata.effects {
            if !(effect.endorsed || effect.declassified) {
                continue;
            }
            let Some((scrutinee, pattern)) = &effect.selected_by else {
                continue;
            };
            let root = scrutinee.split('.').next().unwrap_or(scrutinee.as_str());
            if low_integrity_bindings.contains(&root) {
                let crossing = if effect.declassified {
                    "declassify"
                } else {
                    "endorse"
                };
                diagnostics.push(Diagnostic {
                    span: effect.span,
                    message: format!(
                        "integrity violation in rule `{rule}`: a {crossing} crossing is selected by \
                         the low-integrity discriminant `{scrutinee}` (arm `{pattern}`) — an \
                         attacker-steered crossing (NMIF-on-the-selector)",
                        rule = rule.name,
                    ),
                    suggestion: Some(format!(
                        "do not branch a crossing on untrusted `{scrutinee}`; gate the `case` on \
                         high-integrity data, or endorse `{root}` before the `case`"
                    )),
                    related: Vec::new(),
                });
            }
        }
    }
    diagnostics
}

/// The principal-ceiling check (DR-0031 / DR-0028 D3): an agent acts-for the
/// principal it serves and no further, so every resource the program reads must be
/// one the principal's role is cleared for — otherwise the agent would exceed the
/// user's clearance. `principal_role` is the resolved acts-for role of the current
/// principal (the public bottom for an unknown one). Only meaningful when governance
/// declared parties; the caller gates on `has_parties`.
pub fn check_principal_ceiling(
    ir: &IrProgram,
    verified: &VerifiedEnvelope,
    principal_role: &str,
) -> Vec<Diagnostic> {
    let envelope = verified.envelope();
    let mut diagnostics = Vec::new();
    let mut flagged: BTreeSet<String> = BTreeSet::new();
    for rule in &ir.rules {
        let mut reads: Vec<(&str, whipplescript_parser::SourceSpan)> = Vec::new();
        for effect in &rule.metadata.effects {
            if let Some(resource) = &effect.resource {
                if matches!(
                    effect.kind,
                    IrEffectKind::FileRead | IrEffectKind::FileImport
                ) {
                    reads.push((resource.as_str(), effect.span));
                }
            }
            for grant in &effect.access_grants {
                if grant.operations.iter().any(|op| is_read_op(&op.operation)) {
                    reads.push((grant.resource.as_str(), effect.span));
                }
            }
        }
        for when in &rule.whens {
            if let Some(rest) = when.pattern.trim_start().strip_prefix("message from ") {
                if let Some(channel) = rest.split_whitespace().next() {
                    reads.push((channel, when.span));
                }
            }
        }
        for (src, span) in reads {
            let required = envelope.reader_authority(src);
            if !envelope.can_act(principal_role, required)
                && flagged.insert(format!("{}:{src}", rule.name))
            {
                diagnostics.push(Diagnostic {
                    span,
                    message: format!(
                        "identity-ceiling violation in rule `{rule}`: the agent acts-for \
                         `{principal_role}` but reads `{src}` (readable by {required}), exceeding the \
                         user's clearance (DR-0028 D3)",
                        rule = rule.name,
                    ),
                    suggestion: Some(format!(
                        "the principal role `{principal_role}` is not cleared for `{src}`; serve a user \
                         whose role acts-for {required}, or do not read `{src}`"
                    )),
                    related: Vec::new(),
                });
            }
        }
    }
    diagnostics
}

/// The information-flow SURFACE of a workflow (DR-0029 X1): every resource, egress
/// sink, and principal it can touch, as sorted ids. The producer of a `@tool`
/// package declares this and attests `ifc_surface(ir) ⊆ declared`; the consumer
/// checks the surface refines its envelope (no element is an ungoverned door).
/// Mirrors the resource collection of `check_with_envelope`, so the surface is
/// exactly the set of handles the checker would treat as a source or sink.
pub fn ifc_surface(ir: &IrProgram) -> Vec<String> {
    let mut surface: BTreeSet<String> = BTreeSet::new();
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            if let Some(resource) = &effect.resource {
                surface.insert(resource.clone());
            }
            for grant in &effect.access_grants {
                surface.insert(grant.resource.clone());
            }
            if effect.kind == IrEffectKind::HumanAsk {
                surface.insert("human".to_owned());
            }
            if matches!(
                effect.kind,
                IrEffectKind::EventEmit | IrEffectKind::EventNotify
            ) {
                surface.insert("stream".to_owned());
            }
            if effect.kind == IrEffectKind::AgentTell {
                if let Some(provider) = effect
                    .agent
                    .as_deref()
                    .and_then(|name| ir.agents.iter().find(|a| a.name == name))
                    .and_then(|a| a.provider.as_deref())
                {
                    surface.insert(provider.to_owned());
                }
            }
        }
        for write in &rule.metadata.fact_writes {
            surface.insert(format!(
                "fact:{}",
                write.strip_prefix("schema:").unwrap_or(write)
            ));
        }
        for when in &rule.whens {
            let pattern = when.pattern.trim_start();
            if let Some(rest) = pattern.strip_prefix("message from ") {
                if let Some(channel) = rest.split_whitespace().next() {
                    surface.insert(channel.to_owned());
                }
            }
            if pattern.starts_with("human answered") {
                surface.insert("human".to_owned());
            }
        }
    }
    surface.into_iter().collect()
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
    /// Principals (providers/humans) cleared for non-public data — readers, not
    /// protected data (H5).
    pub cleared_principals: Vec<String>,
    /// The workflow's full IFC surface (DR-0029 X1): every door it opens.
    pub surface: Vec<String>,
}

pub fn governance_report(ir: &IrProgram, verified: &VerifiedEnvelope) -> GovernanceReport {
    let envelope = verified.envelope();
    // protected = governed resources whose reader authority is not public, EXCLUDING
    // principals (a provider/human is a cleared reader, not protected data) (H5).
    let mut protected: Vec<String> = envelope
        .readers
        .keys()
        .filter(|name| !envelope.principals.contains(*name))
        .cloned()
        .collect();
    protected.sort();
    // Principals (providers/humans) cleared for non-public data, listed separately.
    let mut cleared_principals: Vec<String> = envelope
        .principals
        .iter()
        .filter(|name| envelope.readers.contains_key(*name))
        .map(|name| format!("{name} (cleared for {})", envelope.reader_authority(name)))
        .collect();
    cleared_principals.sort();
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
    // Source-declared crossings (DR-0027 I-IFC3): an `endorsed` marker in a rule
    // makes the integrity crossing visible at the source point. Surfaced alongside
    // the governance grants so the audit picture is complete — where a crossing is
    // claimed, not only that one is authorized.
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            let at = effect.binding.as_deref().unwrap_or("coerce");
            if effect.endorsed {
                trusted_surface.push(format!("endorsed (source) at rule `{}` ({at})", rule.name));
            }
            if effect.declassified {
                trusted_surface.push(format!(
                    "declassified (source) at rule `{}` ({at})",
                    rule.name
                ));
            }
        }
    }
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
        .filter(|resource| !envelope.governed.contains(envelope.resolve(resource)))
        .collect();
    GovernanceReport {
        protected,
        violations,
        coverage_gaps,
        trusted_surface,
        cleared_principals,
        surface: ifc_surface(ir),
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
        if !self.cleared_principals.is_empty() {
            out.push_str("  cleared principals (providers/humans, not protected data):\n");
            for principal in &self.cleared_principals {
                out.push_str(&format!("    - {principal}\n"));
            }
        }
        if self.surface.is_empty() {
            out.push_str("  information-flow surface: none (opens no doors)\n");
        } else {
            out.push_str("  information-flow surface (every door this workflow opens):\n");
            for door in &self.surface {
                out.push_str(&format!("    - {door}\n"));
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
    fn emitted_event_is_a_stream_egress_door() {
        // reading a confidential store and emitting an event publishes it to the
        // durable log, observed by the DR-0026 session-event stream and telemetry
        // export (E2): `emit` is a sink `stream`, default public.
        let program = r#"@service
workflow IfcEmit

output result R
class R { ok bool }
class Ticket { id string  status "open" }

signal app.ping { note string }
file store ledger { root "./ledger"  allow read ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule emit_it
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from ledger at "data.txt" as loaded
  after loaded succeeds as file {
    emit signal app.ping to ticket.id {
      note file.content
    } as sent
  }
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
                    && d.message.contains("stream")),
            "confidential read + emit should leak to the event stream, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn human_ask_question_is_an_egress_door() {
        // reading a confidential store and then asking a human egresses the turn's
        // context to the human (E2): `askHuman` is a sink, `human` defaults public.
        let program = r##"@service
workflow IfcAsk

output result R
class R { ok bool }
class Ticket { id string  status "open" }

file store ledger { root "./ledger"  allow read ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule ask
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from ledger at "data.txt" as loaded
  after loaded succeeds as file {
    askHuman as review choices ["ok", "no"] "Decide on: {{ file.content }}"
    done ticket
  }
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
                    && d.message.contains("human")),
            "confidential read + askHuman should leak to the human, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
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
    fn imported_tool_surface_must_be_governed() {
        // an imported @tool's surface must be covered by the consumer envelope; an
        // ungoverned door is flagged fail-closed (DR-0029 X1/X8).
        let envelope =
            Envelope::from_json(r#"{ "resources": { "crm": { "reader": "Operator" } } }"#)
                .expect("valid");
        let verified = VerifiedEnvelope::for_test(envelope);
        let imported = vec![
            ("ToolA".to_owned(), vec!["crm".to_owned()]),
            (
                "ToolB".to_owned(),
                vec!["crm".to_owned(), "secret_db".to_owned()],
            ),
        ];
        let gaps = imported_surface_gaps(&imported, &verified);
        assert!(
            !gaps.iter().any(|(tool, _)| *tool == "ToolA"),
            "a fully-governed tool surface has no gap: {gaps:?}"
        );
        let tool_b = gaps
            .iter()
            .find(|(tool, _)| *tool == "ToolB")
            .expect("ToolB opens an ungoverned door");
        assert_eq!(tool_b.1, vec!["secret_db"]);
    }

    #[test]
    fn ifc_surface_enumerates_every_door() {
        // the surface (X1) is the full set of resources/egresses/principals a
        // workflow touches — files, channels, the fact-base, providers, etc.
        let program = r##"@service
workflow IfcSurface

output result R
class R { ok bool }
class Note { id string }
class Ticket { id string  status "open" }

agent coder { provider fixture  profile "p"  capacity 1 }
file store crm { root "./crm"  allow read ["**"] }
channel out { provider slack  destination "#out" }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from crm at "c.json" as loaded
  after loaded succeeds as file {
    send via out { text "hi" } as sent
    after sent succeeds {
      record Note { id "n1" }
    }
  }
}
"##;
        let ir = compile_program(program).ir.expect("compiles");
        let surface = ifc_surface(&ir);
        for expected in ["crm", "out", "fact:Note"] {
            assert!(
                surface.iter().any(|d| d == expected),
                "surface should include `{expected}`, got: {surface:?}"
            );
        }
    }

    #[test]
    fn source_endorsed_marker_surfaces_in_trusted_surface() {
        // a `coerce ... endorsed` source marker (I-IFC3) appears in the guarantee
        // report's trusted surface, tied to its rule, so the crossing is visible at
        // the source point — not only in governance.
        let program = r#"@service
workflow EndorseSurface

output result R
class R { ok bool }
class Reviewed { verdict string }
class Ticket { id string  status "open" }

coerce review(content string) -> Reviewed {
  prompt "classify {{ content }}"
}

table seed as Ticket [ { id "T1"  status "open" } ]

rule triage
  when Ticket as ticket where ticket.status == "open"
=> {
  coerce review("hi") as verdict endorsed
  after verdict succeeds as v {
    complete result { ok true }
  }
}
"#;
        let ir = compile_program(program).ir.expect("compiles");
        let envelope = Envelope::from_json(r#"{ "resources": {} }"#).expect("valid");
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            report
                .trusted_surface
                .iter()
                .any(|c| c.contains("endorsed (source)") && c.contains("triage")),
            "source endorse should be surfaced: {:?}",
            report.trusted_surface
        );
    }

    #[test]
    fn principal_ceiling_caps_reads_to_the_users_clearance() {
        // ledger is Operator-readable; an agent acting-for Requester (who does not
        // act-for Operator) may not read it — exceeding the user's clearance (D3).
        let env = Envelope::from_dsl(
            "grant file_store ledger -> file:/srv/ledger.db readable by Operator\n\
             party alice : Operator\n\
             party bob : Requester\n",
        )
        .expect("valid");
        assert!(env.has_parties());
        assert_eq!(env.role_for_principal("bob"), "Requester");
        // an unknown principal is the public bottom (fail-closed).
        assert_eq!(env.role_for_principal("mallory"), "public");
        let ir = ir_with_grants(READ_LEDGER);
        let verified = VerifiedEnvelope::for_test(env);
        // Requester is capped — refused the Operator read.
        let requester = check_principal_ceiling(&ir, &verified, "Requester");
        assert!(
            requester
                .iter()
                .any(|d| d.message.contains("identity-ceiling") && d.message.contains("ledger")),
            "Requester should be capped: {:?}",
            requester.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        // Operator is cleared — no ceiling violation.
        let operator = check_principal_ceiling(&ir, &verified, "Operator");
        assert!(
            operator.is_empty(),
            "Operator should be cleared: {:?}",
            operator.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_handles_bound_to_same_address_share_the_label() {
        // E5: handles `a` and `b` bound to the same `kind:address` are the same
        // resource and share its label — governance reasons about the real resource,
        // not the script name.
        let env = Envelope::from_dsl(
            "grant file_store a -> file:/srv/crm.db readable by Operator\n\
             grant file_store b -> file:/srv/crm.db readable by Operator\n\
             grant channel out -> smtp:out public\n",
        )
        .expect("valid");
        assert_eq!(env.reader_authority("a"), "Operator");
        assert_eq!(env.reader_authority("b"), "Operator");
        // both leak to the public channel — they are the same secret.
        assert!(env.leaks("a", "out"));
        assert!(env.leaks("b", "out"));
        // a declassify naming handle `a` clears `b` too (same address).
        let with = Envelope::from_dsl(
            "grant file_store a -> file:/srv/crm.db readable by Operator\n\
             grant file_store b -> file:/srv/crm.db readable by Operator\n\
             grant channel out -> smtp:out readable by Requester\n\
             grant declassify a to Requester\n",
        )
        .expect("valid");
        assert!(!with.leaks("a", "out"));
        assert!(
            !with.leaks("b", "out"),
            "declassify of handle `a` should clear `b` too (same address)"
        );
    }

    #[test]
    fn provider_principal_is_not_listed_as_protected_data() {
        // a provider cleared for Operator data is a principal (a reader), not a
        // secret — it must not appear under "protected resources" (H5).
        let envelope = Envelope::from_dsl(
            "grant file_store crm -> file:/srv/crm readable by Operator\n\
             grant provider fixture -> selfhost:llama readable by Operator\n",
        )
        .expect("valid");
        let ir = ir_with_grants(READ_LEDGER);
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        // the report names the canonical kind:address identity, not the handle (E5).
        assert!(report.protected.contains(&"file:/srv/crm".to_owned()));
        assert!(
            !report.protected.iter().any(|name| name == "selfhost:llama"),
            "a provider principal must not be listed as protected data: {:?}",
            report.protected
        );
        assert!(
            report
                .cleared_principals
                .iter()
                .any(|line| line.contains("selfhost:llama")),
            "the cleared provider should be listed as a principal: {:?}",
            report.cleared_principals
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
