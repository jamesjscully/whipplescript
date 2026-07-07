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

use whipplescript_parser::{
    Diagnostic, IrEffectKind, IrEffectNode, IrProgram, IrRule, IrWorkflowContractKind,
};

/// The bottom reader-authority: data readable by `public` is readable by anyone,
/// and `public` itself holds no authority above itself.
const PUBLIC: &str = "public";

type FieldReadMap = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

/// The party-relative confidentiality projection of the governance envelope
/// (DR-0027 I-IFC1): each governed resource has a **reader authority** (a role);
/// the secret is readable by any party that acts-for that role. The delegation
/// context is the acts-for edge set, closed reflexive-transitively by `can_act`.
pub struct Envelope {
    /// resource handle -> reader-authority SET (a set of compartments; absent or
    /// empty = `public`, the bottom). A party may read the resource iff it acts-for
    /// EVERY compartment — the intersection of up-sets (DR-0027 E6, the set form
    /// proven in `models/lean/Whipple/ReaderSets.lean`). A single-compartment label
    /// is the leaf case, behaving exactly as the role it replaces.
    readers: BTreeMap<String, BTreeSet<String>>,
    governed: BTreeSet<String>,
    /// acts-for edges `(p, q)`: `p` acts-for `q` (has at least `q`'s authority).
    deleg: Vec<(String, String)>,
    /// declassify grants `(resource, role)`: `resource` may be released to any
    /// party that acts-for `role`. These are the audited trusted-surface holes.
    declassify: Vec<(String, String)>,
    /// integrity (writer/vouching) authority SET per resource (absent or empty =
    /// `public`, the untrusted bottom). A control sink requiring integrity set `ws`
    /// accepts data only from a source whose integrity set DOMINATES `ws` — provides
    /// some voucher acting-for each required one (DR-0027 I-IFC1/E6, the dual of the
    /// reader axis).
    integrity: BTreeMap<String, BTreeSet<String>>,
    /// endorse grants `(resource, role)`: `resource`'s data may be raised to `role`
    /// integrity — the audited integrity-axis crossing.
    endorse: Vec<(String, String)>,
    /// signal resources (`signal:<name>`) governance marks INTERNAL (H8 stage b): an
    /// internal signal is an internal channel, NOT an external entry point, so its
    /// integrity at a receiver is DERIVED from its emitters (carriage) rather than
    /// defaulting low, and an external `whip signal` injection of it is refused (no
    /// laundering). A signal absent here is an external-entry point (stage a).
    internal_signals: BTreeSet<String>,
    /// workflow-invoke resources (`invoke:<name>`) governance marks INTERNAL (E2):
    /// the target is attested as a bundle-private workflow, not a cross-boundary
    /// invocation endpoint.
    internal_workflows: BTreeSet<String>,
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
        let mut internal_signals = BTreeSet::new();
        let mut internal_workflows = BTreeSet::new();
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
                let mut reader_set = parse_role_set(label, "reader");
                // back-compat: `confidential: true` is the single-compartment label
                // `{confidential}` (the original binary form).
                if reader_set.is_empty()
                    && label
                        .get("confidential")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                {
                    reader_set.insert("confidential".to_owned());
                }
                if !reader_set.is_empty() {
                    readers.insert(name.clone(), reader_set);
                }
                let writer_set = parse_role_set(label, "writer");
                if !writer_set.is_empty() {
                    integrity.insert(name.clone(), writer_set);
                }
                // a signal resource marked `internal` derives its integrity from its
                // emitters (H8 stage b) rather than defaulting to the external-entry
                // low.
                if label
                    .get("internal")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    if name.starts_with("invoke:") {
                        internal_workflows.insert(name.clone());
                    } else {
                        internal_signals.insert(name.clone());
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
            internal_signals,
            internal_workflows,
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
        let mut internal_signals = BTreeSet::new();
        let mut internal_workflows = BTreeSet::new();
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
            // `readable by <Role>[, <Role>...]` sets the reader-authority SET (E6):
            // every compartment listed after `by`, up to the `from` keyword or the
            // end. Roles may be comma- or space-separated; `public` is dropped.
            if let Some(by) = label.iter().position(|tok| *tok == "by") {
                let until = label
                    .iter()
                    .skip(by + 1)
                    .position(|tok| *tok == "from")
                    .map_or(label.len(), |rel| by + 1 + rel);
                let roles = collect_role_set(&label[by + 1..until]);
                if !roles.is_empty() {
                    readers.insert(address.clone(), roles);
                }
            }
            // `internal` marks a signal an internal channel (H8 stage b): its
            // integrity is derived from its emitters, not the external-entry low.
            if label.contains(&"internal") {
                if address.starts_with("invoke:") {
                    internal_workflows.insert(address.clone());
                } else {
                    internal_signals.insert(address.clone());
                }
            }
            // `from <Role>[, <Role>...]` sets the integrity (vouching) SET: the
            // compartments after `from` to the end.
            if let Some(from) = label.iter().position(|tok| *tok == "from") {
                let roles = collect_role_set(&label[from + 1..]);
                if !roles.is_empty() {
                    integrity.insert(address, roles);
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
            internal_signals,
            internal_workflows,
            address_of,
            party_of,
        })
    }

    /// The canonical signed-artifact JSON: every governed resource with its reader
    /// authority, plus the delegation edges, all sorted (deterministic hash).
    pub fn to_canonical_json(&self) -> String {
        let mut resources = serde_json::Map::new();
        for name in &self.governed {
            // reader/writer are emitted as sorted compartment ARRAYS (E6); a public
            // label is the empty array. The BTreeSet iterates in sorted order, so the
            // canonical form is deterministic (stable signing hash).
            let mut entry = serde_json::json!({
                "reader": self.reader_set(name).into_iter().collect::<Vec<_>>(),
                "writer": self.integrity_set(name).into_iter().collect::<Vec<_>>(),
            });
            if self.principals.contains(name) {
                entry["principal"] = serde_json::Value::Bool(true);
            }
            if self.internal_signals.contains(name) || self.internal_workflows.contains(name) {
                entry["internal"] = serde_json::Value::Bool(true);
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

    /// The reader-authority SET of a resource; the empty set (`public`, the bottom)
    /// if unlabeled. A party may read iff it acts-for every compartment.
    fn reader_set(&self, resource: &str) -> BTreeSet<String> {
        self.readers
            .get(self.resolve(resource))
            .cloned()
            .unwrap_or_default()
    }

    /// A reader label rendered for diagnostics: `public` for the empty set, else the
    /// compartments joined by `, `.
    fn reader_label(&self, resource: &str) -> String {
        label_text(&self.reader_set(resource))
    }

    /// The reader-authority of a `redact <source> keep [..]` PROJECTION: the JOIN
    /// (union of compartments — a combined value is readable only by a party cleared
    /// for every part) of the kept fields' per-field labels. Per-field labels are
    /// envelope resources keyed `<schema>.<field>` (e.g. `Customer.ssn`), so an
    /// unlabeled field is public and `keep`ing only public fields yields a public
    /// projection — exactly the per-field non-interference proven in
    /// `models/lean/Whipple/Redaction.lean` (`canRead_redact`) and
    /// `models/maude/infoflow-redaction.maude` (`projReaders`). Keeping every field
    /// recovers the whole-record join (`redact_keep_all` = the opaque box). The
    /// dropped fields never contribute — they are physically removed at runtime, so
    /// they cannot leak.
    fn projected_reader_set(&self, schema: &str, keep: &[String]) -> BTreeSet<String> {
        let mut readers = BTreeSet::new();
        for field in keep {
            readers.extend(self.reader_set(&format!("{schema}.{field}")));
        }
        readers
    }

    /// `provider` DOMINATES `required` iff every required compartment is covered by
    /// some provider compartment (via acts-for) — the leak/inject decision, proven
    /// sound in `ReaderSets.lean` (`leak_safe`). An empty `required` is vacuously
    /// dominated (a public source never leaks); an empty `provider` dominates only
    /// the empty set (a public sink cannot carry a confidential source).
    fn dominates(&self, provider: &BTreeSet<String>, required: &BTreeSet<String>) -> bool {
        required
            .iter()
            .all(|req| provider.iter().any(|prov| self.can_act(prov, req)))
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
        let sink_readers = self.reader_set(sink);
        if self.dominates(&sink_readers, &self.reader_set(source)) {
            return false;
        }
        // a declassify releases the WHOLE source to `role`: its effective reader
        // requirement drops to the single compartment `{role}`, cleared iff the sink
        // dominates that (the audited escape hatch, DR-0027 I-IFC3).
        for (resource, role) in &self.declassify {
            if self.resolve(resource) == self.resolve(source)
                && self.dominates(&sink_readers, &BTreeSet::from([role.clone()]))
            {
                return false;
            }
        }
        true
    }

    /// The integrity (vouching) authority SET of a resource; the empty set
    /// (`public`, the untrusted bottom) if unlabeled.
    fn integrity_set(&self, resource: &str) -> BTreeSet<String> {
        self.integrity
            .get(self.resolve(resource))
            .cloned()
            .unwrap_or_default()
    }

    /// Whether governance marks `resource` (a `signal:<name>`) an INTERNAL channel
    /// (H8 stage b): its integrity is derived from its emitters, and it may not be
    /// externally injected.
    pub fn is_internal_signal(&self, resource: &str) -> bool {
        self.internal_signals.contains(self.resolve(resource))
    }

    /// Whether governance marks `resource` (an `invoke:<name>`) an INTERNAL
    /// workflow endpoint (E2): the workflow is private to the bundle and should
    /// not be externally nameable.
    pub fn is_internal_workflow(&self, resource: &str) -> bool {
        self.internal_workflows.contains(self.resolve(resource))
    }

    /// Whether this envelope governs `resource`, after applying handle->address
    /// bindings. This is the narrow runtime authority query used by owned-harness
    /// tool enforcement; it does not expose labels or acts-for internals.
    pub fn governs(&self, resource: &str) -> bool {
        self.governed.contains(self.resolve(resource))
    }

    /// An integrity label rendered for diagnostics: `public` for the empty set, else
    /// the compartments joined by `, `.
    fn integrity_label(&self, resource: &str) -> String {
        label_text(&self.integrity_set(resource))
    }

    /// Does reading `read` and writing `write` inject? Untrusted data pollutes a
    /// trusted sink: safe iff `read`'s integrity acts-for `write`'s requirement (the
    /// dual of `leaks`), OR an endorse grant raises `read` to a role that meets the
    /// requirement (the audited integrity crossing, DR-0027 I-IFC3).
    fn injects(&self, read: &str, write: &str) -> bool {
        let requirement = self.integrity_set(write);
        let read_integrity = self.integrity_set(read);
        if self.dominates(&read_integrity, &requirement) {
            return false;
        }
        // an endorse raises `read` to vouch `role`: add it to the provided set and
        // re-check whether the requirement is now met (the audited integrity
        // crossing, DR-0027 I-IFC3).
        for (resource, role) in &self.endorse {
            if self.resolve(resource) == self.resolve(read) {
                let mut raised = read_integrity.clone();
                raised.insert(role.clone());
                if self.dominates(&raised, &requirement) {
                    return false;
                }
            }
        }
        true
    }
}

/// Render a compartment set for diagnostics: `public` (the bottom) when empty, else
/// the sorted compartments joined by `, `.
fn label_text(set: &BTreeSet<String>) -> String {
    if set.is_empty() {
        PUBLIC.to_owned()
    } else {
        set.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// Parse a reader/writer label field into a compartment SET: a JSON string is the
/// single-compartment leaf, a JSON array is the general set. `public` is dropped (it
/// is the bottom, represented by absence/emptiness), so a `["public"]` or `"public"`
/// label is the empty set.
fn parse_role_set(label: &serde_json::Value, key: &str) -> BTreeSet<String> {
    match label.get(key) {
        Some(serde_json::Value::String(role)) if role != PUBLIC => BTreeSet::from([role.clone()]),
        Some(serde_json::Value::Array(roles)) => roles
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|role| *role != PUBLIC)
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// Collect a set of authority roles from DSL tokens, splitting each token on commas
/// (so `Operator,Auditor` and `Operator Auditor` both yield two compartments) and
/// dropping `public` (the bottom).
fn collect_role_set(tokens: &[&str]) -> BTreeSet<String> {
    let mut roles = BTreeSet::new();
    for token in tokens {
        for role in token.split(',') {
            let role = role.trim();
            if !role.is_empty() && role != PUBLIC {
                roles.insert(role.to_owned());
            }
        }
    }
    roles
}

/// Integrity carried by a signal, as a voucher SET; `None` is TOP — fully trusted,
/// the identity for the meet (a signal emitted only by rules that read nothing
/// external). `Some(set)` is the concrete voucher set; `Some(∅)` is the untrusted
/// bottom. The meet (combine) of two integrities is the INTERSECTION of vouchers
/// (data is trusted only as much as its least-trusted input — the E6 integrity dual).
type CarriedIntegrity = Option<BTreeSet<String>>;

/// Render a carried integrity for diagnostics: `trusted (derived)` for TOP, else the
/// voucher set (`public` for the empty/bottom set).
fn carried_label(integrity: &CarriedIntegrity) -> String {
    match integrity {
        None => "trusted (derived)".to_owned(),
        Some(set) => label_text(set),
    }
}

/// The meet of two carried integrities: intersection of voucher sets, with `None`
/// (top) as the identity.
fn meet_integrity(a: CarriedIntegrity, b: CarriedIntegrity) -> CarriedIntegrity {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
    }
}

/// The read sources of a rule (for computing the integrity its `emit`s carry): file
/// reads, turn-grant reads, inbound message channels, signal triggers, and human
/// answers — the same source recognition the rule-level join box uses.
fn rule_read_resources(
    rule: &IrRule,
    signal_names: &BTreeSet<&str>,
    shared_coordination: &BTreeSet<String>,
) -> Vec<String> {
    let mut reads: Vec<String> = Vec::new();
    for effect in &rule.metadata.effects {
        if let Some(resource) = ifc_resource_for_effect(effect, shared_coordination) {
            if matches!(
                effect.kind,
                IrEffectKind::FileRead
                    | IrEffectKind::FileImport
                    | IrEffectKind::LeaseAcquire
                    | IrEffectKind::LedgerAppend
                    | IrEffectKind::CounterConsume
            ) {
                reads.push(resource.to_owned());
            }
        }
        for grant in &effect.access_grants {
            if grant.operations.iter().any(|op| is_read_op(&op.operation)) {
                reads.push(grant.resource.clone());
            }
        }
    }
    for when in &rule.whens {
        let pattern = when.pattern.trim_start();
        if let Some(rest) = pattern.strip_prefix("message from ") {
            if let Some(channel) = rest.split_whitespace().next() {
                reads.push(channel.to_owned());
            }
        }
        if pattern.starts_with("human answered") {
            reads.push("human".to_owned());
        }
        if let Some(name) = pattern.split_whitespace().next() {
            if signal_names.contains(name) {
                reads.push(format!("signal:{name}"));
            }
        }
    }
    reads
}

/// The integrity an `emit` in `rule` carries: the meet (intersection) of the
/// integrity of every source the rule reads. A rule that reads nothing external is
/// TOP (`None`); a rule that reads any untrusted source drops to its meet.
fn carried_integrity_of_rule(
    envelope: &Envelope,
    rule: &IrRule,
    signal_names: &BTreeSet<&str>,
    shared_coordination: &BTreeSet<String>,
) -> CarriedIntegrity {
    let mut acc: CarriedIntegrity = None;
    for src in rule_read_resources(rule, signal_names, shared_coordination) {
        acc = meet_integrity(acc, Some(envelope.integrity_set(&src)));
    }
    acc
}

/// The signal ports a rule emits (`emit signal <name> [to <peer>]` → resource
/// `signal:<name>`). The directed form lowers to `SignalEmit`, the broadcast form to
/// `EventEmit`; both carry the emitter's payload across the boundary.
fn emitted_signal_ports(rule: &IrRule) -> Vec<String> {
    rule.metadata
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect.kind,
                IrEffectKind::EventEmit | IrEffectKind::SignalEmit
            )
        })
        .filter_map(|effect| effect.resource.clone())
        .filter(|resource| resource.starts_with("signal:"))
        .collect()
}

/// The DERIVED integrity of each signal that some rule emits (H8 stage b carriage):
/// `signal:<name>` → the meet, over its emitting rules, of the integrity each emit
/// carries. The receiver's `when <name>` reads this instead of the external-entry
/// default — so an internal signal inherits its emitters' trust automatically.
///
/// Spans MULTIPLE programs: the consumer plus every imported `@tool` (DR-0029
/// cross-package carriage). The label is always computed under the CONSUMER's
/// envelope from the pinned source, so it is the consumer's own governance reasoning
/// about the imported emitter — no producer label attestation needed; the producer
/// need only attest the surface (which names the emit port). Signals with no emitter
/// in any program are absent (the caller falls back to the envelope label).
fn derived_signal_integrity(
    programs: &[&IrProgram],
    envelope: &Envelope,
) -> BTreeMap<String, CarriedIntegrity> {
    let mut derived: BTreeMap<String, CarriedIntegrity> = BTreeMap::new();
    for ir in programs {
        let signal_names: BTreeSet<&str> = ir.events.iter().map(|e| e.name.as_str()).collect();
        let shared_coordination = shared_coordination_resources(ir);
        for rule in &ir.rules {
            let ports = emitted_signal_ports(rule);
            if ports.is_empty() {
                continue;
            }
            let carried =
                carried_integrity_of_rule(envelope, rule, &signal_names, &shared_coordination);
            for port in ports {
                let merged = match derived.remove(&port) {
                    None => carried.clone(),
                    Some(prev) => meet_integrity(prev, carried.clone()),
                };
                derived.insert(port, merged);
            }
        }
    }
    derived
}

/// The binding name introduced by `… as <binding>` in a `when` pattern, if any.
fn binding_after_as(pattern: &str) -> Option<&str> {
    let mut tokens = pattern.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "as" {
            return tokens.next();
        }
    }
    None
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

fn is_coordination_effect(kind: &IrEffectKind) -> bool {
    matches!(
        kind,
        IrEffectKind::LeaseAcquire | IrEffectKind::LedgerAppend | IrEffectKind::CounterConsume
    )
}

fn shared_coordination_resources(ir: &IrProgram) -> BTreeSet<String> {
    if !ir.shared_coordination_usage.is_empty() {
        return ir
            .shared_coordination_usage
            .iter()
            .filter(|usage| usage.workflow_principals.len() >= 2)
            .map(|usage| usage.resource.clone())
            .collect();
    }

    ir.leases
        .iter()
        .filter(|lease| lease.shared)
        .map(|lease| format!("resource:{}", lease.name))
        .chain(
            ir.ledgers
                .iter()
                .filter(|ledger| ledger.shared)
                .map(|ledger| format!("resource:{}", ledger.name)),
        )
        .chain(
            ir.counters
                .iter()
                .filter(|counter| counter.shared)
                .map(|counter| format!("resource:{}", counter.name)),
        )
        .collect()
}

fn ifc_resource_for_effect<'a>(
    effect: &'a IrEffectNode,
    shared_coordination: &BTreeSet<String>,
) -> Option<&'a str> {
    let resource = effect.resource.as_deref()?;
    if is_coordination_effect(&effect.kind) && !shared_coordination.contains(resource) {
        return None;
    }
    Some(resource)
}

fn selected_effect_integrity_sinks(
    effect: &IrEffectNode,
    shared_coordination: &BTreeSet<String>,
) -> Vec<String> {
    let mut sinks = Vec::new();
    if let Some(resource) = ifc_resource_for_effect(effect, shared_coordination) {
        if matches!(
            effect.kind,
            IrEffectKind::FileWrite
                | IrEffectKind::FileExport
                | IrEffectKind::CapabilityCall
                | IrEffectKind::LeaseAcquire
                | IrEffectKind::LedgerAppend
                | IrEffectKind::CounterConsume
        ) {
            sinks.push(resource.to_owned());
        }
    }
    for grant in &effect.access_grants {
        if grant
            .operations
            .iter()
            .any(|op| is_egress_op(&op.operation))
        {
            sinks.push(grant.resource.clone());
        }
    }
    if effect.kind == IrEffectKind::HumanAsk {
        sinks.push("human".to_owned());
    }
    if matches!(
        effect.kind,
        IrEffectKind::EventEmit | IrEffectKind::SignalEmit
    ) {
        sinks.push("stream".to_owned());
    }
    sinks.sort();
    sinks.dedup();
    sinks
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
    /// Present and authentic — an unsigned dev policy, or signed + verified. Boxed:
    /// the verified envelope is much larger than the other variants.
    Verified(Box<VerifiedEnvelope>),
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
            Ok(envelope) => EnvelopeStatus::Verified(Box::new(VerifiedEnvelope { envelope })),
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

    /// Whether the verified envelope governs `resource`, after applying
    /// handle->address bindings.
    pub fn governs(&self, resource: &str) -> bool {
        self.envelope.governs(resource)
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

pub fn internal_workflow_from_env(resources: &[String]) -> Result<bool, String> {
    match VerifiedEnvelope::load_from_env() {
        EnvelopeStatus::Ungoverned => Ok(false),
        EnvelopeStatus::Rejected(message) => Err(message),
        EnvelopeStatus::Verified(verified) => Ok(resources
            .iter()
            .any(|resource| verified.envelope().is_internal_workflow(resource))),
    }
}

/// Run the IFC check if a governance envelope is configured; otherwise no
/// constraints apply (dev mode) and this returns no diagnostics. Routes through the
/// trust boundary: a signed policy is verified first, and the whip agent refuses to
/// enforce a tampered one.
pub fn check_ifc_program(ir: &IrProgram) -> Vec<Diagnostic> {
    check_ifc_program_with_imports(ir, &[])
}

/// `check_ifc_program` aware of imported `@tool` programs, so cross-package signal
/// carriage (DR-0029 / H8 stage b) folds imported emit ports into the consumer's
/// derived signal integrity.
pub fn check_ifc_program_with_imports(ir: &IrProgram, imports: &[IrProgram]) -> Vec<Diagnostic> {
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
            let mut diagnostics = check_with_envelope_imports(ir, &verified, imports);
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

/// Whether the env-configured governed envelope marks signal `<name>` an INTERNAL
/// channel (H8 stage b). `whip signal` uses this to refuse an external injection of
/// an internal signal: an internal channel carries its emitter's integrity and must
/// not be sourced from outside (the W6 no-laundering principle). Ungoverned/absent or
/// a rejected envelope → `false` (the gradual model imposes nothing in dev mode).
pub fn signal_is_internal(signal_name: &str) -> bool {
    match VerifiedEnvelope::load_from_env() {
        EnvelopeStatus::Verified(verified) => verified
            .envelope()
            .is_internal_signal(&format!("signal:{signal_name}")),
        _ => false,
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
    let mut gaps = Vec::new();
    for (tool, surface) in imported {
        let ungoverned: Vec<&str> = surface
            .iter()
            .map(String::as_str)
            .filter(|door| !verified.governs(door))
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
    check_with_envelope_imports(ir, verified, &[])
}

/// `check_with_envelope` aware of imported `@tool` programs (DR-0029): an imported
/// tool's `emit signal X` contributes its carried integrity to the consumer's
/// `signal:X`, so a cross-package internal signal propagates the emitter's trust just
/// as an in-program one does. `imports` are the pinned tool IRs, compiled by the
/// consumer; labels are computed under the consumer's envelope.
/// The read-source resources a program touches across ALL its rules — the opaque
/// tool-level join box (DR-0030 X2 baseline A: the result carries the join of
/// everything the tool reads).
fn program_read_resources(ir: &IrProgram) -> Vec<String> {
    let signal_names: BTreeSet<&str> = ir.events.iter().map(|e| e.name.as_str()).collect();
    let shared_coordination = shared_coordination_resources(ir);
    let mut reads: BTreeSet<String> = BTreeSet::new();
    for rule in &ir.rules {
        reads.extend(rule_read_resources(
            rule,
            &signal_names,
            &shared_coordination,
        ));
    }
    reads.into_iter().collect()
}

/// The read resources an imported tool's RESULT provably depends on (DR-0030 X2
/// Direction A, the reach refinement — computed consumer-side from the pinned tool
/// source, since structural reach is label-agnostic and the consumer recompiles the
/// source anyway). The result depends only on the reads of the rules that **reach a
/// completing rule** — itself plus every transitive upstream rule whose recorded fact
/// it consumes. A resource read ONLY by rules that never feed a `complete` is
/// `independent_of` the result (a proven non-interference, `noReach`) and is dropped,
/// so the result carries a smaller join than the whole-tool baseline. Whole-result v1:
/// reads are attributed at rule granularity (no per-field value-flow), so the cut is
/// the rule-dependency graph. Falls back to all reads if the tool never completes.
fn result_dependency_reads(tool: &IrProgram) -> Vec<String> {
    let completing: BTreeSet<&str> = tool
        .rules
        .iter()
        .filter(|rule| !rule.metadata.terminal_completes.is_empty())
        .map(|rule| rule.name.as_str())
        .collect();
    if completing.is_empty() {
        return program_read_resources(tool);
    }
    reach_reads_from(tool, completing).into_iter().collect()
}

/// The reads feeding a `seed` set of rules: the seed plus every rule that
/// transitively feeds one via a recorded fact (reverse-reachability over the
/// producer→consumer fact-dependency graph), unioned over their read resources.
/// This is the reach primitive behind both the whole-result signature
/// (`result_dependency_reads`, seeded by the completing rules) and the per-field
/// signatures (`result_field_dependency_reads` / milestone D3′, seeded by a single
/// fact's producers).
fn reach_reads_from(tool: &IrProgram, seed: BTreeSet<&str>) -> BTreeSet<String> {
    let mut contributing = seed;
    loop {
        let mut added = false;
        for dep in &tool.rule_dependencies {
            if contributing.contains(dep.consumer.as_str())
                && contributing.insert(dep.producer.as_str())
            {
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let signal_names: BTreeSet<&str> = tool.events.iter().map(|e| e.name.as_str()).collect();
    let shared_coordination = shared_coordination_resources(tool);
    let mut reads: BTreeSet<String> = BTreeSet::new();
    for rule in &tool.rules {
        if contributing.contains(rule.name.as_str()) {
            reads.extend(rule_read_resources(
                rule,
                &signal_names,
                &shared_coordination,
            ));
        }
    }
    reads
}

/// An egress field's flow signature: the egress binding/name, the field name, and
/// the reads reaching that field — the PER-FIELD refinement of that egress's whole
/// dependency reach (DR-0030 X2 v2 / D3′). The refinement is at FACT granularity
/// and preserves the rule-level opaque box (I-IFC2): the emitting rule's OWN reads reach every
/// field, and only the BETWEEN-rule fact provenance is refined per field. A field
/// root that is a DIRECT `when <Fact> as root` binding contributes only that fact's
/// producer reach; any other root (a within-rule derived binding, or a `when`
/// binding of an inbound/external fact with no internal producer) has opaque
/// provenance and FALLS BACK to the egress's whole reach — the fail-closed core, so
/// a field reach is always a subset of the whole egress reach and never
/// under-reports. Proven in `models/maude/infoflow-field-signature.maude`.
///
/// CONSUMER-SIDE NOTE (documented boundary, not a gap): the per-field signature is
/// producer-side audit transparency. It cannot yet RELAX a cross-package consumer
/// check, because the only consumer path — an agent turn that may call an imported
/// tool (`tell <agent> with tools […]`) — folds the tool result into an OPAQUE turn
/// (we can't see which result fields it reads), so the turn conservatively inherits
/// the whole-result reach. Per-field ENFORCEMENT needs a non-opaque consumer (turn
/// field-access grants, or IFC-tracked `invoke` result-field access); until then
/// the field signature is exposed for audit, and the whole-result join still governs.
fn field_dependency_reads(
    tool: &IrProgram,
    whole: BTreeSet<String>,
    select: fn(&IrRule) -> &FieldReadMap,
) -> Vec<(String, String, Vec<String>)> {
    let signal_names: BTreeSet<&str> = tool.events.iter().map(|e| e.name.as_str()).collect();
    let shared_coordination = shared_coordination_resources(tool);
    // egress -> field -> reads, unioned across every emitting/completing rule.
    let mut per_field: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for rule in &tool.rules {
        let field_reads = select(rule);
        if field_reads.is_empty() {
            continue;
        }
        let own: BTreeSet<String> = rule_read_resources(rule, &signal_names, &shared_coordination)
            .into_iter()
            .collect();
        let when_facts = when_binding_facts(rule);
        for (egress, fields) in field_reads {
            for (field, roots) in fields {
                let mut reads = own.clone();
                for root in roots {
                    match when_facts.get(root.as_str()) {
                        // A direct `when <Fact> as root` binding: precise. The reads
                        // feeding this field are the producers of that fact and their
                        // upstreams. An inbound/external fact with no internal producer
                        // yields an empty seed-reach → falls back to the whole reach.
                        Some(fact) => {
                            let producers: BTreeSet<&str> = tool
                                .rules
                                .iter()
                                .filter(|r| r.metadata.fact_writes.iter().any(|w| w == fact))
                                .map(|r| r.name.as_str())
                                .collect();
                            if producers.is_empty() {
                                reads.clone_from(&whole);
                            } else {
                                reads.extend(reach_reads_from(tool, producers));
                            }
                        }
                        // A within-rule derived binding: opaque provenance, fall back
                        // to the whole-result reach (fail-closed).
                        None => reads.clone_from(&whole),
                    }
                }
                per_field
                    .entry((egress.clone(), field.clone()))
                    .or_default()
                    .extend(reads);
            }
        }
    }
    per_field
        .into_iter()
        .map(|((binding, field), reads)| (binding, field, reads.into_iter().collect()))
        .collect()
}

fn result_field_dependency_reads(tool: &IrProgram) -> Vec<(String, String, Vec<String>)> {
    let whole: BTreeSet<String> = result_dependency_reads(tool).into_iter().collect();
    field_dependency_reads(tool, whole, |rule| &rule.metadata.complete_field_reads)
}

fn milestone_field_dependency_reads(tool: &IrProgram) -> Vec<(String, String, Vec<String>)> {
    let emitting: BTreeSet<&str> = tool
        .rules
        .iter()
        .filter(|rule| !rule.metadata.milestone_field_reads.is_empty())
        .map(|rule| rule.name.as_str())
        .collect();
    if emitting.is_empty() {
        return Vec::new();
    }
    let whole = reach_reads_from(tool, emitting);
    field_dependency_reads(tool, whole, |rule| &rule.metadata.milestone_field_reads)
}

/// A rule's `when <Fact> as <binding>` bindings, mapped `binding -> schema:<Fact>`
/// (the fact string the rule-dependency graph uses). Only patterns that bind a
/// name and whose head is a schema fact are captured; message/signal/human and
/// bindingless triggers are omitted, so their roots take the conservative
/// whole-result fallback in `result_field_dependency_reads`.
fn when_binding_facts(rule: &IrRule) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for when in &rule.whens {
        let pattern = when.pattern.trim();
        // The binding is the tail `… as <binding>`.
        let Some((head, binding)) = pattern.rsplit_once(" as ") else {
            continue;
        };
        let binding = binding.trim();
        if binding.is_empty() || binding.contains(char::is_whitespace) {
            continue;
        }
        // The fact is the head schema name — the first token, before any `{ … }`
        // field pattern. Inbound sources (`message from …`, `human …`) are not
        // schema facts, so their bindings are left to the fallback.
        let head = head.trim();
        if head.starts_with("message from") || head.starts_with("human") {
            continue;
        }
        let Some(schema) = head.split([' ', '{']).next() else {
            continue;
        };
        if schema.is_empty() || !schema.chars().next().is_some_and(|c| c.is_uppercase()) {
            continue;
        }
        out.insert(binding.to_owned(), format!("schema:{schema}"));
    }
    out
}

/// Flags the redact static refinement's CONFIDENTIALITY check on fully-redacted
/// egresses (`complete` bindings, `fact:<Schema>` record sinks, or `send`
/// channels): a sink whose payload references ONLY redaction outputs (each with a
/// resolvable source schema) must have its own label dominate the JOIN of those
/// projections' kept-field labels (`projected_reader_set`) — else keeping a
/// too-sensitive field is flagged (naming it). This is PURELY ADDITIVE: it does
/// NOT exempt the egress from the conservative read×sink leak check. The kept
/// fields carry data derived from the rule's READS, whose provenance the schema
/// field labels do not capture, so exempting the egress from those reads was
/// unsound (a confirmed under-taint: a redacted egress of confidential-resource
/// data released with no grant). Releasing resource-read-derived data at a lower
/// label is a declassification and still requires a `grant declassify` (honoured by
/// the conservative loop). The proven model (`Redaction.lean`) covers the
/// projection algebra given per-field labels; it does not cover read provenance —
/// which is exactly why the exemption slipped past it. (The value-flow engine that
/// tracks per-field provenance is the real refinement; this keeps the tree sound.)
fn flag_redacted_egress_projections(
    candidates: &[String],
    rule: &IrRule,
    envelope: &Envelope,
    span: whipplescript_parser::SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let projected_for = |binding: &str| -> Option<BTreeSet<String>> {
        let redaction = rule
            .metadata
            .redactions
            .iter()
            .find(|redaction| redaction.binding == binding)?;
        let schema = redaction.source_schema.as_deref()?;
        Some(envelope.projected_reader_set(schema, &redaction.keep))
    };
    for sink in candidates {
        let roots = rule.metadata.egress_payload_reads.get(sink);
        let fully_redacted = roots.is_some_and(|roots| {
            !roots.is_empty() && roots.iter().all(|r| projected_for(r).is_some())
        });
        if !fully_redacted {
            continue;
        }
        let projected: BTreeSet<String> = roots
            .into_iter()
            .flatten()
            .filter_map(|root| projected_for(root))
            .flatten()
            .collect();
        let sink_readers = envelope.reader_set(sink);
        if !envelope.dominates(&sink_readers, &projected) {
            // Name exactly which kept fields the sink cannot read, and suggest the
            // safe keep-set — the sound "auto-suggest" form of auto-redaction, which
            // keeps the crossing explicit (the author still narrows the `keep` list).
            let mut offending: BTreeSet<String> = BTreeSet::new();
            let mut safe: Vec<String> = Vec::new();
            for root in roots.into_iter().flatten() {
                let Some(redaction) = rule.metadata.redactions.iter().find(|r| &r.binding == root)
                else {
                    continue;
                };
                let Some(schema) = redaction.source_schema.as_deref() else {
                    continue;
                };
                for field in &redaction.keep {
                    let field_label = envelope.reader_set(&format!("{schema}.{field}"));
                    if envelope.dominates(&sink_readers, &field_label) {
                        safe.push(field.clone());
                    } else {
                        offending.insert(field.clone());
                    }
                }
            }
            let suggestion = if offending.is_empty() {
                format!("clear the sink with `grant … -> {sink} readable by <role>`")
            } else {
                let dropped = offending.iter().cloned().collect::<Vec<_>>().join(", ");
                let keep = safe.join(", ");
                format!(
                    "drop the field(s) `{dropped}` the sink cannot read (keep only [{keep}]), or \
                     clear the sink with `grant … -> {sink} readable by <role>`"
                )
            };
            diagnostics.push(Diagnostic {
                span,
                message: format!(
                    "information-flow violation in rule `{rule}`: the redacted egress `{sink}` \
                     still carries fields readable by {proj}, but `{sink}` is readable by {have}",
                    rule = rule.name,
                    proj = label_text(&projected),
                    have = envelope.reader_label(sink),
                ),
                suggestion: Some(suggestion),
                related: Vec::new(),
            });
        }
    }
}

pub fn check_with_envelope_imports(
    ir: &IrProgram,
    verified: &VerifiedEnvelope,
    imports: &[IrProgram],
) -> Vec<Diagnostic> {
    let envelope = verified.envelope();
    // The declared signal names, so a `when <Signal> as e` trigger is recognized as
    // an inbound read source (H8). Source recognition is uniform: a signal is a
    // tracked read of `signal:<name>`, integrity envelope-declared, default public
    // (the untrusted/fail-closed bottom) — exactly as channels work — so an
    // unrecognized signal can no longer fail OPEN past a governed envelope.
    let signal_names: BTreeSet<&str> = ir.events.iter().map(|e| e.name.as_str()).collect();
    let shared_coordination = shared_coordination_resources(ir);
    // H8 stage b: the integrity each emitted signal carries to its receivers (the
    // meet over its emitters, across the consumer AND every imported tool). An
    // `internal`-marked signal reads this instead of the external-entry default, so
    // internal flows propagate the emitter's trust automatically.
    let programs: Vec<&IrProgram> = std::iter::once(ir).chain(imports.iter()).collect();
    let derived = derived_signal_integrity(&programs, envelope);
    // A `@tool` workflow's `complete result` crosses a PACKAGE boundary: its invoker
    // is a future consumer whose clearance is party-relative and unknown at the
    // producer, so the result is governed CONSUMER-side by the flow signature
    // (DR-0030 X2), never as a local sink here. A `@service`/top-level workflow's
    // result returns to the operator in the SAME governance domain, so its
    // `complete result` IS a local egress sink (the invoker boundary), governed below.
    let is_tool = ir
        .source_tags
        .iter()
        .any(|tag| tag.target_kind == "workflow" && tag.name == "tool");
    let mut diagnostics = Vec::new();
    for rule in &ir.rules {
        // Collect reads and writes across the whole rule (the rule-level join box):
        // both `with access to` turn grants AND direct file effects in the body.
        let mut reads: Vec<&str> = Vec::new();
        let mut writes: Vec<&str> = Vec::new();
        let mut span = None;
        for effect in &rule.metadata.effects {
            if let Some(resource) = ifc_resource_for_effect(effect, &shared_coordination) {
                match effect.kind {
                    IrEffectKind::FileRead | IrEffectKind::FileImport => {
                        reads.push(resource);
                        span.get_or_insert(effect.span);
                    }
                    IrEffectKind::FileWrite | IrEffectKind::FileExport => {
                        writes.push(resource);
                        span.get_or_insert(effect.span);
                    }
                    // `send via <channel>` lowers to a capability call carrying the
                    // channel as its resource; it is an egress sink.
                    IrEffectKind::CapabilityCall => {
                        writes.push(resource);
                        span.get_or_insert(effect.span);
                    }
                    // Shared coordination is bidirectional: the mutation writes the
                    // resource, and the outcome/discriminant reads it.
                    IrEffectKind::LeaseAcquire
                    | IrEffectKind::LedgerAppend
                    | IrEffectKind::CounterConsume => {
                        reads.push(resource);
                        writes.push(resource);
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
                IrEffectKind::EventEmit | IrEffectKind::SignalEmit
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
                                    rr = envelope.reader_label(resource),
                                    pr = envelope.reader_label(provider),
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
        let record_candidates: Vec<String> = rule
            .metadata
            .fact_writes
            .iter()
            .map(|write| format!("fact:{}", write.strip_prefix("schema:").unwrap_or(write)))
            .collect();
        // `complete result {…}` returns a value to the workflow's invoker — an egress
        // sink at the invoker boundary (DR-0030 X2, top-level half). For a
        // `@service`/top-level workflow the invoker is the operator in the same
        // governance domain, so the result is a local confidentiality sink named by the
        // output binding, default public/fail-closed and cleared by a grant
        // (`grant <kind> <handle> -> <binding> readable by <role>`). A `@tool` result
        // is NOT here (it crosses a package boundary, governed consumer-side).
        //
        // DR-0027 redact (the static refinement): an egress (a `complete` OR a
        // `record`) whose payload references ONLY redaction outputs is FULLY-REDACTED.
        // The runtime physically projects each such binding to its kept fields, so the
        // egress carries only those — its confidentiality is the kept fields' per-field
        // label join (`projected_reader_set`), NOT the rule's whole read set. Such an
        // egress is governed by its projected label here and EXCLUDED from the
        // conservative read×sink loop; a mixed or unresolved egress stays conservative.
        let redact_span = span.unwrap_or(whipplescript_parser::SourceSpan { start: 0, end: 0 });
        let result_sinks: Vec<String> = if is_tool {
            Vec::new()
        } else {
            rule.metadata.terminal_completes.clone()
        };
        let record_sinks = record_candidates;
        let milestone_sinks: Vec<String> = rule
            .metadata
            .milestone_field_reads
            .keys()
            .map(|name| format!("milestone:{name}"))
            .collect();
        // Redact refinement (PURELY ADDITIVE — DR-0027): a fully-redacted egress must
        // have its sink dominate the kept fields' per-field label join. This does NOT
        // exempt the egress from the conservative read×sink leak below — the kept
        // fields carry read-derived data whose provenance the schema labels don't
        // capture, so exempting was an under-taint; releasing read-derived data at a
        // lower label needs a `grant declassify` (honoured by the conservative loop).
        let redact_candidates: Vec<String> = result_sinks
            .iter()
            .cloned()
            .chain(record_sinks.iter().cloned())
            .chain(milestone_sinks.iter().cloned())
            .chain(writes.iter().map(|sink| (*sink).to_owned()))
            .collect();
        flag_redacted_egress_projections(
            &redact_candidates,
            rule,
            envelope,
            redact_span,
            &mut diagnostics,
        );
        // Bounded-type egresses (`record <T> from <src>`, DR-0027 auto-redaction): the
        // recorded fact keeps exactly `T`'s fields, checked against those fields'
        // per-field label join. Also purely additive (no read exemption).
        for bounded in &rule.metadata.bounded_egresses {
            let projected = envelope.projected_reader_set(&bounded.source_schema, &bounded.keep);
            let sink_readers = envelope.reader_set(&bounded.sink);
            if envelope.dominates(&sink_readers, &projected) {
                continue;
            }
            let offending: Vec<String> = bounded
                .keep
                .iter()
                .filter(|field| {
                    !envelope.dominates(
                        &sink_readers,
                        &envelope.reader_set(&format!("{}.{}", bounded.source_schema, field)),
                    )
                })
                .cloned()
                .collect();
            diagnostics.push(Diagnostic {
                span: redact_span,
                message: format!(
                    "information-flow violation in rule `{rule}`: the bounded-type egress \
                     `{sink}` carries fields readable by {proj}, but `{sink}` is readable by {have}",
                    rule = rule.name,
                    sink = bounded.sink,
                    proj = label_text(&projected),
                    have = envelope.reader_label(&bounded.sink),
                ),
                suggestion: Some(format!(
                    "remove the field(s) `{dropped}` from the target type, or clear the sink with \
                     `grant … -> {sink} readable by <role>`",
                    dropped = offending.join(", "),
                    sink = bounded.sink,
                )),
                related: Vec::new(),
            });
        }
        // DR-0030 X2 (cross-package): a `tell <agent>` turn whose agent may call an
        // imported `@tool` (DR-0025 `tools [...]`) can pull that tool's result into the
        // turn — and the tool may read confidential/low-integrity data the consumer
        // never touched directly. So the imported tool's RESULT reads (resolved in the
        // shared governance envelope) become read SOURCES of the turn's rule, and a
        // tool whose result then flows to a consumer sink is caught on both axes.
        // `result_dependency_reads` is the Direction-A reach refinement: only the reads
        // that reach a completing rule (the rest are `independent_of` the result and
        // dropped). It degrades to the whole-tool join box when the tool's result
        // depends on everything. Imported tools are matched to the agent's `tools` list
        // by workflow name.
        let mut tool_result_reads: Vec<String> = Vec::new();
        for effect in &rule.metadata.effects {
            if effect.kind != IrEffectKind::AgentTell {
                continue;
            }
            let Some(agent_name) = &effect.agent else {
                continue;
            };
            let Some(agent) = ir.agents.iter().find(|a| &a.name == agent_name) else {
                continue;
            };
            for tool_name in &agent.tools {
                if let Some(tool) = imports.iter().find(|t| &t.workflow == tool_name) {
                    tool_result_reads.extend(result_dependency_reads(tool));
                }
            }
        }
        // Inbound `when message from <channel>` delivers attacker-controllable
        // content: the channel is a low-integrity READ source (and public
        // confidentiality), so untrusted inbound data driving a more-trusted sink is
        // caught as an injection (H3). The IR pattern is `message from <channel>`.
        let mut message_reads: Vec<&str> = Vec::new();
        // `when <Signal> as e` triggers: a signal is injected from outside the
        // instance (an operator/peer `whip signal`, a directed `emit signal X to`),
        // so it is an inbound read source `signal:<name>` (H8). Owned because the id
        // is the prefixed name, not a borrow of the pattern.
        let mut signal_reads: Vec<String> = Vec::new();
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
            // a trigger whose head is a declared signal name reads that signal.
            if let Some(name) = pattern.split_whitespace().next() {
                if signal_names.contains(name) {
                    signal_reads.push(format!("signal:{name}"));
                }
            }
        }
        let report_span = span.unwrap_or(whipplescript_parser::SourceSpan { start: 0, end: 0 });
        // An internal signal reads its DERIVED integrity (carriage); every other
        // source reads the envelope label. `None` integrity is TOP (never injects).
        let internal_signal: BTreeSet<&str> = signal_reads
            .iter()
            .map(String::as_str)
            .filter(|sig| envelope.is_internal_signal(sig))
            .collect();
        let source_integrity = |src: &str| -> CarriedIntegrity {
            if internal_signal.contains(src) {
                derived
                    .get(src)
                    .cloned()
                    .unwrap_or_else(|| Some(envelope.integrity_set(src)))
            } else {
                Some(envelope.integrity_set(src))
            }
        };
        let mut leak: Option<(String, String)> = None;
        let mut inject: Option<(String, String, String)> = None;
        for src in reads
            .iter()
            .copied()
            .chain(message_reads.iter().copied())
            .chain(signal_reads.iter().map(String::as_str))
            .chain(tool_result_reads.iter().map(String::as_str))
        {
            let src_integrity = source_integrity(src);
            for sink in writes
                .iter()
                .copied()
                .chain(record_sinks.iter().map(String::as_str))
                .chain(milestone_sinks.iter().map(String::as_str))
                .chain(result_sinks.iter().map(String::as_str))
            {
                if leak.is_none() && envelope.leaks(src, sink) {
                    leak = Some((src.to_owned(), sink.to_owned()));
                }
                if inject.is_none() {
                    // an internal signal carries its derived integrity (no endorse
                    // hatch); every other source uses the envelope label + endorse.
                    let injects = if internal_signal.contains(src) {
                        match &src_integrity {
                            None => false,
                            Some(set) => !envelope.dominates(set, &envelope.integrity_set(sink)),
                        }
                    } else {
                        envelope.injects(src, sink)
                    };
                    if injects {
                        inject = Some((
                            src.to_owned(),
                            sink.to_owned(),
                            carried_label(&src_integrity),
                        ));
                    }
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
                    src_reader = envelope.reader_label(&src),
                    sink_reader = envelope.reader_label(&sink),
                ),
                suggestion: Some(format!(
                    "self-serve (no grant needed): separate the contexts — read `{src}` in a \
                     distinct turn and pass only a bounded result. escalate (needs governance): \
                     request `grant declassify {src} to <role cleared for {sink}>`"
                )),
                related: Vec::new(),
            });
        }
        if let Some((src, sink, src_int)) = inject {
            diagnostics.push(Diagnostic {
                span: report_span,
                message: format!(
                    "integrity violation in rule `{rule}`: it may let untrusted `{src}` (integrity \
                     {src_int}) influence `{sink}` (requires integrity {sink_int}), an injection \
                     into a more-trusted sink",
                    rule = rule.name,
                    sink_int = envelope.integrity_label(&sink),
                ),
                suggestion: Some(format!(
                    "self-serve (no grant needed): do not let `{src}` influence `{sink}` — gate \
                     the sink on trusted data. escalate (needs governance): request `grant endorse \
                     {src} to <role>` to vouch the source"
                )),
                related: Vec::new(),
            });
        }

        // NMIF-on-the-selector (DR §5.6 / §7.4): a crossing (`endorsed`/`declassified`)
        // inside a `case <disc> { … }` arm whose discriminant is low-integrity is
        // rejected — the attacker must not steer which declassify/endorse runs. The
        // discriminant is low-integrity when its root binding comes from a
        // low-integrity `when` source: an inbound message / a human answer, or (H8) a
        // signal trigger the envelope does not vouch (a Family-B signal discriminant
        // gating a crossing — the §5.6 channel-2 case the uniform recognition makes
        // live). A signal vouched by governance (`signal:<name> from <Role>`) is
        // high-integrity and may steer a crossing.
        let low_integrity_bindings: Vec<&str> = rule
            .whens
            .iter()
            .filter_map(|when| {
                let pattern = when.pattern.trim_start();
                if pattern.starts_with("message from ") || pattern.starts_with("human answered") {
                    return binding_after_as(pattern);
                }
                if let Some(name) = pattern.split_whitespace().next() {
                    if signal_names.contains(name)
                        && envelope.integrity_set(&format!("signal:{name}")).is_empty()
                    {
                        return binding_after_as(pattern);
                    }
                }
                None
            })
            .collect();
        let input_roots: BTreeSet<&str> = ir
            .workflow_contracts
            .iter()
            .filter(|contract| matches!(contract.kind, IrWorkflowContractKind::Input))
            .map(|contract| contract.name.as_str())
            .collect();
        let invoke_selector_port = format!("invoke:{}", ir.workflow);
        for effect in &rule.metadata.effects {
            let Some((scrutinee, pattern)) = &effect.selected_by else {
                continue;
            };
            let root = scrutinee.split('.').next().unwrap_or(scrutinee.as_str());
            let selector_is_invoke_input = input_roots.contains(root);
            let selector_integrity = if selector_is_invoke_input {
                Some(envelope.integrity_set(&invoke_selector_port))
            } else if low_integrity_bindings.contains(&root) {
                Some(BTreeSet::new())
            } else {
                None
            };
            if selector_integrity.as_ref().is_some_and(BTreeSet::is_empty)
                && (effect.endorsed || effect.declassified)
            {
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
            let Some(selector_integrity) = selector_integrity else {
                continue;
            };
            if !selector_is_invoke_input {
                continue;
            }
            for sink in selected_effect_integrity_sinks(effect, &shared_coordination) {
                let required = envelope.integrity_set(&sink);
                if envelope.dominates(&selector_integrity, &required) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    span: effect.span,
                    message: format!(
                        "integrity violation in rule `{rule}`: the low-integrity selector \
                         `{scrutinee}` (arm `{pattern}`) controls `{sink}`, which requires \
                         integrity {sink_int} (NMIF-on-invoke-selector)",
                        rule = rule.name,
                        sink_int = envelope.integrity_label(&sink),
                    ),
                    suggestion: Some(format!(
                        "do not let `{scrutinee}` select a higher-integrity effect; vouch the \
                         inbound invoke port `{invoke_selector_port}` with `grant invoke ... from \
                         <role>`, or move the effect outside the untrusted `case`"
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
    let signal_names: BTreeSet<&str> = ir.events.iter().map(|e| e.name.as_str()).collect();
    let shared_coordination = shared_coordination_resources(ir);
    let mut diagnostics = Vec::new();
    let mut flagged: BTreeSet<String> = BTreeSet::new();
    for rule in &ir.rules {
        let mut reads: Vec<(String, whipplescript_parser::SourceSpan)> = Vec::new();
        for effect in &rule.metadata.effects {
            if let Some(resource) = ifc_resource_for_effect(effect, &shared_coordination) {
                if matches!(
                    effect.kind,
                    IrEffectKind::FileRead
                        | IrEffectKind::FileImport
                        | IrEffectKind::LeaseAcquire
                        | IrEffectKind::LedgerAppend
                        | IrEffectKind::CounterConsume
                ) {
                    reads.push((resource.to_owned(), effect.span));
                }
            }
            for grant in &effect.access_grants {
                if grant.operations.iter().any(|op| is_read_op(&op.operation)) {
                    reads.push((grant.resource.clone(), effect.span));
                }
            }
        }
        for when in &rule.whens {
            let pattern = when.pattern.trim_start();
            if let Some(rest) = pattern.strip_prefix("message from ") {
                if let Some(channel) = rest.split_whitespace().next() {
                    reads.push((channel.to_owned(), when.span));
                }
            }
            // a `when <Signal>` trigger reads `signal:<name>` (H8); the principal
            // must be cleared for the signal's reader set too.
            if let Some(name) = pattern.split_whitespace().next() {
                if signal_names.contains(name) {
                    reads.push((format!("signal:{name}"), when.span));
                }
            }
        }
        for (src, span) in reads {
            let src = src.as_str();
            let required = envelope.reader_set(src);
            // the principal must be cleared for EVERY compartment of the source (it
            // can read iff it acts-for the whole reader set — `canRead`).
            let cleared = required.iter().all(|r| envelope.can_act(principal_role, r));
            if !cleared && flagged.insert(format!("{}:{src}", rule.name)) {
                let required = envelope.reader_label(src);
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
    let signal_names: BTreeSet<&str> = ir.events.iter().map(|e| e.name.as_str()).collect();
    let shared_coordination = shared_coordination_resources(ir);
    let mut surface: BTreeSet<String> = BTreeSet::new();
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            if let Some(resource) = ifc_resource_for_effect(effect, &shared_coordination) {
                surface.insert(resource.to_owned());
            }
            if let Some(target) = &effect.workflow_target {
                surface.insert(format!("invoke:{target}"));
            }
            for grant in &effect.access_grants {
                surface.insert(grant.resource.clone());
            }
            if effect.kind == IrEffectKind::HumanAsk {
                surface.insert("human".to_owned());
            }
            if matches!(
                effect.kind,
                IrEffectKind::EventEmit | IrEffectKind::SignalEmit
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
            // a `when <Signal>` trigger opens the `signal:<name>` door (H8).
            if let Some(name) = pattern.split_whitespace().next() {
                if signal_names.contains(name) {
                    surface.insert(format!("signal:{name}"));
                }
            }
        }
    }
    surface.into_iter().collect()
}

/// The IT-facing guarantee report (`gov compile`, DR-0028): what a governance
/// config guarantees and the risks it leaves. Surfaces per-resource guaranteed
/// invariants (the exact confidentiality/integrity proven on every rule), the count
/// of IFC violations the config catches, flagged risks (touched-but-ungoverned
/// resources, fail-closed to public/low), the audited trusted surface (declassify /
/// endorse crossings to review), cleared principals (H5), and the full door surface.
pub struct GovernanceReport {
    /// Per-resource guaranteed invariants (DR-0028): for each governed resource, the
    /// exact confidentiality/integrity the checker guarantees on every rule — not a
    /// generic line. The "guaranteed invariants" half of the guarantee report.
    pub invariants: Vec<String>,
    /// Flagged risks (DR-0028): coverage gaps reframed as risks the operator must
    /// confirm (each defaults to public + low-integrity, fail-closed). Audited
    /// crossings — the other risk class — are surfaced in `trusted_surface`.
    pub flagged_risks: Vec<String>,
    pub violations: usize,
    /// The audited trusted surface: each crossing, tagged by axis —
    /// `declassify <resource> -> <role>` and `endorse <resource> -> <role>`.
    pub trusted_surface: Vec<String>,
    /// Principals (providers/humans) cleared for non-public data — readers, not
    /// protected data (H5).
    pub cleared_principals: Vec<String>,
    /// The workflow's full IFC surface (DR-0029 X1): every door it opens.
    pub surface: Vec<String>,
    /// The per-field flow signature (DR-0030 X2 v2): for each `complete <binding>`
    /// result field, the reads reaching it, refined at fact granularity. Producer-
    /// side audit transparency — a consumer of `result.<field>` inherits only these
    /// reads. Empty when the workflow completes no result fields.
    pub flow_signature: Vec<String>,
}

pub fn governance_report(ir: &IrProgram, verified: &VerifiedEnvelope) -> GovernanceReport {
    let envelope = verified.envelope();
    // Principals (providers/humans) cleared for non-public data, listed separately.
    let mut cleared_principals: Vec<String> = envelope
        .principals
        .iter()
        .filter(|name| envelope.readers.contains_key(*name))
        .map(|name| format!("{name} (cleared for {})", envelope.reader_label(name)))
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
    let shared_coordination = shared_coordination_resources(ir);
    for rule in &ir.rules {
        for effect in &rule.metadata.effects {
            if let Some(resource) = ifc_resource_for_effect(effect, &shared_coordination) {
                touched.insert(resource.to_owned());
            }
            for grant in &effect.access_grants {
                touched.insert(grant.resource.clone());
            }
        }
    }
    let coverage_gaps: Vec<String> = touched
        .into_iter()
        .filter(|resource| !envelope.governed.contains(envelope.resolve(resource)))
        .collect();
    // Per-resource guaranteed invariants: every governed resource (on either axis,
    // excluding principals) gets its exact guarantee, so the report states what is
    // proven, not a generic blanket line. A confidentiality-labelled resource may not
    // flow to a sink not cleared for its reader set; an integrity-labelled one may not
    // be influenced by data below its writer set. Both axes shown when both are set.
    let mut invariant_names: BTreeSet<&String> = envelope.readers.keys().collect();
    invariant_names.extend(envelope.integrity.keys());
    let invariants: Vec<String> = invariant_names
        .into_iter()
        .filter(|name| !envelope.principals.contains(*name))
        .filter_map(|name| {
            let mut clauses: Vec<String> = Vec::new();
            if !envelope.reader_set(name).is_empty() {
                clauses.push(format!(
                    "may not flow to a sink not cleared for {} (unless an audited declassify clears it)",
                    envelope.reader_label(name)
                ));
            }
            if !envelope.integrity_set(name).is_empty() {
                clauses.push(format!(
                    "may not be influenced by data below {} (unless an audited endorse vouches it)",
                    envelope.integrity_label(name)
                ));
            }
            if clauses.is_empty() {
                None
            } else {
                Some(format!("{name}: {}", clauses.join("; ")))
            }
        })
        .collect();
    // Flagged risks: a touched-but-ungoverned resource is a risk the operator must
    // confirm — it defaults to public + low-integrity (fail-closed), so the checker
    // proves nothing about it. (Audited crossings are the other risk class, shown in
    // their own trusted-surface section so each downgrade is reviewable.)
    let flagged_risks: Vec<String> = coverage_gaps
        .iter()
        .map(|resource| {
            format!(
                "{resource}: touched but not labelled by governance — treated as public + \
                 low-integrity (fail-closed). Confirm it holds nothing confidential and feeds no \
                 trusted sink, or add a `grant` for it."
            )
        })
        .collect();
    // The per-field flow signature: for each result or milestone field, the reads
    // reaching it (fact-granular). A field with no reaching reads is stated as
    // `independent` — an audited non-interference claim the invoker can rely on.
    let flow_signature: Vec<String> = result_field_dependency_reads(ir)
        .into_iter()
        .chain(
            milestone_field_dependency_reads(ir)
                .into_iter()
                .map(|(milestone, field, reads)| (format!("milestone:{milestone}"), field, reads)),
        )
        .map(|(binding, field, reads)| {
            if reads.is_empty() {
                format!("{binding}.{field}: independent of every governed read")
            } else {
                format!("{binding}.{field} carries reads: {}", reads.join(", "))
            }
        })
        .collect();
    GovernanceReport {
        invariants,
        flagged_risks,
        violations,
        trusted_surface,
        cleared_principals,
        surface: ifc_surface(ir),
        flow_signature,
    }
}

impl GovernanceReport {
    /// Render the report as IT-legible text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("information-flow guarantee report\n");
        if self.invariants.is_empty() {
            out.push_str("  guaranteed invariants: none (no resource labelled)\n");
        } else {
            out.push_str("  guaranteed invariants (proven by the checker on every rule):\n");
            for invariant in &self.invariants {
                out.push_str(&format!("    - {invariant}\n"));
            }
        }
        out.push_str(&format!(
            "  violations caught in this program: {}\n",
            self.violations
        ));
        if self.flagged_risks.is_empty() {
            out.push_str("  flagged risks: none (every touched resource is governed)\n");
        } else {
            out.push_str("  flagged risks (the operator must confirm or govern these):\n");
            for risk in &self.flagged_risks {
                out.push_str(&format!("    - {risk}\n"));
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
        if !self.flow_signature.is_empty() {
            out.push_str(
                "  result/milestone flow signature (per field, the reads a consumer inherits, \
                 fact-granular):\n",
            );
            for field in &self.flow_signature {
                out.push_str(&format!("    - {field}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whipplescript_parser::{compile_program, compile_program_with_root};

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
        // a turn reading confidential data is fine when BOTH egress boundaries are
        // cleared: the agent's `fixture` provider (the model the turn ships context
        // to) and `result` (the workflow's invoker — `complete result` is an egress to
        // it, DR-0030 X2 top-level). With both cleared for confidential data there is
        // no leak.
        let envelope = Envelope::from_json(
            r#"{ "resources": {
                "ledger": { "confidential": true },
                "fixture": { "reader": "confidential" },
                "result": { "reader": "confidential" }
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
        assert_eq!(from_dsl.reader_label("ledger"), "Operator");
        assert_eq!(from_dsl.reader_label("outbox"), "public");
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

    fn coordination_counter_program(shared: bool) -> String {
        let shared = if shared { "  shared\n" } else { "" };
        format!(
            r#"
@service
workflow SharedCoordIfc

output result Done

class Done {{
  note string
}}

class Customer {{
  id string
}}

counter budget {{
{shared}  key Customer
  cap 1
  reset daily
}}

rule seed
  when started
=> {{
  record Customer {{
    id "cust"
  }}
}}

rule spend
  when Customer as c
=> {{
  consume budget for c.id amount 1 as spend

  after spend ok {{
    complete result {{
      note "ok"
    }}
  }}
  after spend over {{
    complete result {{
      note "over"
    }}
  }}
}}
"#
        )
    }

    fn contended_coordination_counter_program() -> &'static str {
        r#"
class Done {
  note string
}

class Customer {
  id string
}

counter budget {
  shared
  key Customer
  cap 1
  reset daily
}

workflow SharedCoordIfc {
  output result Done

  rule seed
    when started
  => {
    record Customer {
      id "cust"
    }
  }

  rule spend
    when Customer as c
  => {
    consume budget for c.id amount 1 as spend

    after spend ok {
      complete result {
        note "ok"
      }
    }
    after spend over {
      complete result {
        note "over"
      }
    }
  }
}

workflow OtherCoordUser {
  output result Done

  rule seed
    when started
  => {
    record Customer {
      id "other"
    }
  }

  rule spend
    when Customer as c
  => {
    consume budget for c.id amount 1 as spend

    after spend ok {
      complete result {
        note "ok"
      }
    }
    after spend over {
      complete result {
        note "over"
      }
    }
  }
}
"#
    }

    #[test]
    fn shared_coordination_outcome_is_a_confidential_read_source() {
        let ir = compile_program_with_root(
            contended_coordination_counter_program(),
            Some("SharedCoordIfc"),
        )
        .ir
        .expect("compiles");
        let envelope = Envelope::from_dsl(
            "grant coordination budget -> resource:budget readable by Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics.iter().any(|d| {
                d.message.contains("information-flow violation")
                    && d.message.contains("resource:budget")
                    && d.message.contains("result")
            }),
            "shared coordination outcome should be checked as a read source, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn shared_coordination_outcome_may_flow_to_a_cleared_sink() {
        let ir = compile_program_with_root(
            contended_coordination_counter_program(),
            Some("SharedCoordIfc"),
        )
        .ir
        .expect("compiles");
        let envelope = Envelope::from_dsl(
            "grant coordination budget -> resource:budget readable by Operator\n\
             grant output result -> result readable by Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")),
            "cleared result should accept shared coordination outcome, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn partitioned_coordination_is_not_a_cross_principal_ifc_source() {
        let ir = compile_program(&coordination_counter_program(false))
            .ir
            .expect("compiles");
        let envelope = Envelope::from_dsl(
            "grant coordination budget -> resource:budget readable by Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics.is_empty(),
            "partitioned self-coordination should stay out of IFC, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            !ifc_surface(&ir).contains(&"resource:budget".to_owned()),
            "partitioned coordination should not open a shared IFC door"
        );
    }

    #[test]
    fn single_principal_shared_coordination_is_not_a_cross_principal_ifc_source() {
        let ir = compile_program(&coordination_counter_program(true))
            .ir
            .expect("compiles");
        let envelope = Envelope::from_dsl(
            "grant coordination budget -> resource:budget readable by Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics.is_empty(),
            "single-principal shared coordination should stay unlabeled, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            !ifc_surface(&ir).contains(&"resource:budget".to_owned()),
            "single-principal shared coordination should not open a cross-principal door"
        );
    }

    #[test]
    fn shared_coordination_is_in_the_ifc_surface() {
        let ir = compile_program_with_root(
            contended_coordination_counter_program(),
            Some("SharedCoordIfc"),
        )
        .ir
        .expect("compiles");
        assert!(
            ifc_surface(&ir).contains(&"resource:budget".to_owned()),
            "shared coordination should be surfaced"
        );
    }

    #[test]
    fn envelope_tracks_internal_workflow_markers() {
        let env =
            Envelope::from_dsl("grant workflow child -> invoke:Child internal\n").expect("valid");
        assert!(env.is_internal_workflow("invoke:Child"));
        assert!(!env.is_internal_signal("invoke:Child"));

        let canonical = env.to_canonical_json();
        let round_trip = Envelope::from_json(&canonical).expect("canonical envelope");
        assert!(round_trip.is_internal_workflow("invoke:Child"));
        assert!(!round_trip.is_internal_signal("invoke:Child"));
    }

    #[test]
    fn reader_set_requires_clearance_for_every_compartment() {
        // E6: a resource whose label is the SET {Bank, Email} is readable only by a
        // party cleared for BOTH. operator acts-for both; an email-only sink does not.
        let env = Envelope::from_dsl(
            "grant file_store mixed -> file:/srv/mixed readable by Bank,Email\n\
             grant file_store bankbox -> file:/srv/bank readable by Bank\n\
             grant file_store opbox -> file:/srv/op readable by Operator\n\
             grant channel pub -> smtp:pub public\n\
             delegate Operator acts-for Bank\n\
             delegate Operator acts-for Email\n",
        )
        .expect("valid");
        assert_eq!(env.reader_label("mixed"), "Bank, Email");
        // mixed {Bank,Email} -> a Bank-only sink leaks: Email is uncovered.
        assert!(
            env.leaks("mixed", "bankbox"),
            "a Bank-only sink does not dominate a {{Bank,Email}} source"
        );
        // mixed {Bank,Email} -> a public sink leaks (covers nothing).
        assert!(env.leaks("mixed", "pub"));
        // mixed {Bank,Email} -> an Operator sink is SAFE: operator acts-for both, so
        // the singleton {Operator} dominates the whole source set.
        assert!(
            !env.leaks("mixed", "opbox"),
            "Operator acts-for every compartment, so it dominates the set"
        );
        // a Bank-only source -> the {Bank,Email} sink is SAFE: the richer sink set
        // still covers Bank (dominates is monotone in the provider).
        assert!(!env.leaks("bankbox", "mixed"));
    }

    #[test]
    fn integrity_set_requires_every_required_voucher() {
        // E6 dual: a sink requiring the writer SET {Sec, Ops} accepts data only from
        // a source providing a voucher acting-for each. A source vouched only by Sec
        // is rejected; endorsing it to Ops clears it.
        let base = "\
grant file_store sink -> file:/srv/sink from Sec,Ops\n\
grant channel secsrc -> imap:sec from Sec\n";
        let env = Envelope::from_dsl(base).expect("valid");
        assert_eq!(env.integrity_label("sink"), "Ops, Sec");
        // secsrc provides only {Sec}; the sink requires {Sec,Ops} -> Ops unmet -> inject.
        assert!(env.injects("secsrc", "sink"));
        // endorsing secsrc to Ops adds the missing voucher -> no injection.
        let with =
            Envelope::from_dsl(&format!("{base}grant endorse secsrc to Ops\n")).expect("valid");
        assert!(!with.injects("secsrc", "sink"));
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
    fn top_level_complete_result_is_an_egress_to_the_invoker() {
        // DR-0030 X2 (top-level): a @service rule that reads confidential `ledger` and
        // `complete result {…}` returns to its invoker — an egress. With `result`
        // uncleared (default public) this leaks; clearing the invoker fixes it.
        let ir = ir_with_grants(READ_LEDGER);
        let leaks = Envelope::from_json(
            r#"{ "resources": {
            "ledger": { "confidential": true },
            "fixture": { "reader": "confidential" }
        } }"#,
        )
        .expect("valid");
        assert!(
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(leaks))
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("ledger")
                    && d.message.contains("result")),
            "ledger -> result should leak to an uncleared invoker"
        );
        // clearing the invoker (`result` readable for confidential) removes it.
        let cleared = Envelope::from_json(
            r#"{ "resources": {
            "ledger": { "confidential": true },
            "fixture": { "reader": "confidential" },
            "result": { "reader": "confidential" }
        } }"#,
        )
        .expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(cleared))
                .iter()
                .any(|d| d.message.contains("result"))
        );
    }

    const REDACT_COMPLETE: &str = r#"@service
workflow RedactIfc

input customer Customer
output result PublicView

class Customer { id string  ssn string }
class PublicView { who string  detail string }

rule r
  when Customer as c
=> {
  redact c keep [KEEP] as safe
  complete result {
    who safe.id
    detail FIELD
  }
}
"#;

    #[test]
    fn redact_does_not_launder_a_confidential_resource_read() {
        // Regression (confirmed under-taint): a redacted egress must NOT be exempted
        // from the rule's confidential resource READS. Reading confidential `crm`,
        // deriving a typed value, redacting to an unlabelled field, and releasing to a
        // public sink is a declassification of crm-derived data — it must still flag
        // (releasing it needs a `grant declassify`), even though the projected schema
        // label is public. The redact refinement is purely additive, not a read hatch.
        let program = r#"@service
workflow Launder

input trigger Trigger
output result PublicView

class Trigger { k string }
class Customer { id string  ssn string }
class PublicView { x string }

file store crm { root "./crm"  allow read ["**"] }

coerce parse(raw string) -> Customer { prompt "x" }

rule r
  when Trigger as t
=> {
  read text from crm at "customerfile" as raw
  after raw succeeds as loaded {
    coerce parse(loaded.text) as c
    after c succeeds as cust {
      redact cust keep [id] as safe
      complete result { x safe.id }
    }
  }
}
"#;
        let ir = compile_program(program).ir.expect("compiles");
        let envelope =
            Envelope::from_json(r#"{ "resources": { "crm": { "confidential": true } } }"#)
                .expect("valid");
        assert!(
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope))
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("crm")),
            "a redacted egress of confidential-read-derived data must still leak (no read hatch)"
        );
    }

    #[test]
    fn redacted_egress_keeping_only_public_fields_does_not_leak() {
        // DR-0027 redact static refinement: a `complete result` that references ONLY
        // the redacted projection is governed by the kept fields' per-field label,
        // not the whole record. Keeping only public `id` (Customer.ssn is the only
        // confidential field) yields a public projection — no leak, even though the
        // result sink is public.
        let program = REDACT_COMPLETE
            .replace("KEEP", "id")
            .replace("FIELD", "safe.id");
        let ir = compile_program(&program).ir.expect("compiles");
        let envelope = Envelope::from_json(
            r#"{ "resources": { "Customer.ssn": { "reader": "confidential" } } }"#,
        )
        .expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope))
                .iter()
                .any(|d| d.message.contains("result")),
            "a projection keeping only public fields must not leak to a public invoker"
        );
    }

    #[test]
    fn redacted_egress_keeping_a_confidential_field_leaks() {
        // The bite: keeping the confidential `ssn` makes the projection confidential,
        // so the public invoker is not cleared — flagged (the dropped fields are
        // non-interfering, but a KEPT confidential field is not).
        let program = REDACT_COMPLETE
            .replace("KEEP", "id, ssn")
            .replace("FIELD", "safe.ssn");
        let ir = compile_program(&program).ir.expect("compiles");
        let confidential = r#"{ "resources": { "Customer.ssn": { "reader": "confidential" } } }"#;
        let leak_env = Envelope::from_json(confidential).expect("valid");
        let diags = check_with_envelope(&ir, &VerifiedEnvelope::for_test(leak_env));
        let redact_leak = diags
            .iter()
            .find(|d| d.message.contains("redacted egress") && d.message.contains("confidential"))
            .expect("keeping a confidential field must leak to a public invoker");
        // The auto-suggest names the offending field and the safe keep-set.
        let suggestion = redact_leak.suggestion.as_deref().unwrap_or_default();
        assert!(
            suggestion.contains("`ssn`") && suggestion.contains("keep only [id]"),
            "suggestion should name the offending field and the safe keep-set: {suggestion}"
        );
        // Clearing the invoker for `confidential` removes it.
        let cleared = Envelope::from_json(
            r#"{ "resources": {
                "Customer.ssn": { "reader": "confidential" },
                "result": { "reader": "confidential" }
            } }"#,
        )
        .expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(cleared))
                .iter()
                .any(|d| d.message.contains("redacted egress")),
            "clearing the invoker for the kept fields' label removes the leak"
        );
    }

    #[test]
    fn redacted_record_egress_is_governed_by_the_projection() {
        // The same refinement applies to a `record` egress: a recorded fact built
        // only from a redacted projection is governed by the kept fields' label.
        let program = r#"@service
workflow RedactRecord

input customer Customer
output result PublicView

class Customer { id string  ssn string }
class PublicView { ok bool }
class SafeFact { who string }

rule r
  when Customer as c
=> {
  redact c keep [KEEP] as safe
  record SafeFact {
    who FIELD
  }
  complete result { ok true }
}
"#;
        let envelope = r#"{ "resources": { "Customer.ssn": { "reader": "confidential" } } }"#;
        // Keeping only public `id`: the recorded fact is public — no leak.
        let safe = program.replace("KEEP", "id").replace("FIELD", "safe.id");
        let ir = compile_program(&safe).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("SafeFact")),
            "a fact built only from a public projection must not leak"
        );
        // Keeping `ssn`: the recorded fact carries confidential — flagged.
        let leak = program
            .replace("KEEP", "id, ssn")
            .replace("FIELD", "safe.ssn");
        let ir = compile_program(&leak).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("redacted egress")
                    && d.message.contains("fact:SafeFact")),
            "a fact built from a confidential projection must be flagged"
        );
    }

    #[test]
    fn bounded_type_complete_from_is_governed_by_kept_fields() {
        // Bounded-type parity for the invoker egress: `complete T from <src>` keeps
        // exactly the listed shorthand fields, governed by their per-field labels.
        let program = r#"@service
workflow BoundedComplete

input customer Customer
output result PublicView

class Customer { id string  ssn string }
class PublicView { FIELDS }

rule r
  when Customer as cust
=> {
  complete result from cust { KEEP }
}
"#;
        let envelope = r#"{ "resources": { "Customer.ssn": { "reader": "confidential" } } }"#;
        let safe = program.replace("FIELDS", "id string").replace("KEEP", "id");
        let ir = compile_program(&safe).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("result")),
            "a `complete from` keeping only public fields must not leak"
        );
        let leak = program
            .replace("FIELDS", "id string  ssn string")
            .replace("KEEP", "id\n    ssn");
        let ir = compile_program(&leak).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("bounded-type egress") && d.message.contains("result")),
            "a `complete from` keeping a confidential field must be flagged"
        );
    }

    #[test]
    fn bounded_type_record_projection_is_governed_by_kept_fields() {
        // DR-0027 auto-redaction (bounded-type): `record T from <src>` keeps exactly
        // the listed shorthand fields, so it is governed by those fields' per-field
        // labels — no explicit `redact` needed. Keeping only public `id` is safe;
        // also keeping confidential `ssn` is flagged, naming the offending field.
        let program = r#"@service
workflow BoundedRecord

input customer Customer
output result PublicView

class Customer { id string  ssn string }
class PublicView { ok bool }
class SafeFact { FIELDS }

rule r
  when Customer as cust
=> {
  record SafeFact from cust { KEEP }
  complete result { ok true }
}
"#;
        let envelope = r#"{ "resources": { "Customer.ssn": { "reader": "confidential" } } }"#;
        let safe = program.replace("FIELDS", "id string").replace("KEEP", "id");
        let ir = compile_program(&safe).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("SafeFact")),
            "a pure projection keeping only public fields must not leak"
        );
        let leak = program
            .replace("FIELDS", "id string  ssn string")
            .replace("KEEP", "id\n    ssn");
        let ir = compile_program(&leak).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        let diags = check_with_envelope(&ir, &VerifiedEnvelope::for_test(env));
        let leak_diag = diags
            .iter()
            .find(|d| d.message.contains("bounded-type egress") && d.message.contains("SafeFact"))
            .expect("keeping a confidential field in a bounded projection must leak");
        assert!(
            leak_diag
                .suggestion
                .as_deref()
                .unwrap_or_default()
                .contains("`ssn`"),
            "the suggestion should name the offending field: {:?}",
            leak_diag.suggestion
        );
    }

    #[test]
    fn redacted_send_egress_is_governed_by_the_projection() {
        // The refinement also covers a `send via <channel>` egress: a message built
        // only from a redacted projection is governed by the kept fields' label.
        let program = r##"@service
workflow RedactSend

input customer Customer
output result PublicView

class Customer { id string  ssn string }
class PublicView { ok bool }

channel reply {
  provider slack
  destination "#ops"
}

rule r
  when Customer as c
=> {
  redact c keep [KEEP] as safe
  send via reply { text FIELD } as sent
  complete result { ok true }
}
"##;
        let envelope = r#"{ "resources": { "Customer.ssn": { "reader": "confidential" } } }"#;
        let safe = program.replace("KEEP", "id").replace("FIELD", "safe.id");
        let ir = compile_program(&safe).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("reply")),
            "a message built only from a public projection must not leak"
        );
        let leak = program
            .replace("KEEP", "id, ssn")
            .replace("FIELD", "safe.ssn");
        let ir = compile_program(&leak).ir.expect("compiles");
        let env = Envelope::from_json(envelope).expect("valid");
        assert!(
            check_with_envelope(&ir, &VerifiedEnvelope::for_test(env))
                .iter()
                .any(|d| d.message.contains("redacted egress") && d.message.contains("reply")),
            "a message built from a confidential projection must be flagged"
        );
    }

    #[test]
    fn tool_complete_result_is_not_a_local_sink() {
        // a @tool's `complete result` crosses a PACKAGE boundary; its invoker's
        // clearance is party-relative and unknown at the producer, so it is governed
        // consumer-side by the flow signature, NOT as a local sink. So the same
        // confidential read + complete does NOT flag when the program is a @tool.
        let program = r#"@tool
workflow ToolWf

output result R
class R { ok bool }
class Ticket { id string  status "open" }

file store ledger { root "./ledger"  allow read ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from ledger at "data.txt" as loaded
  after loaded succeeds as v {
    complete result { ok true }
  }
}
"#;
        let ir = compile_program(program).ir.expect("compiles");
        let envelope =
            Envelope::from_json(r#"{ "resources": { "ledger": { "confidential": true } } }"#)
                .expect("valid");
        assert!(
            !check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope))
                .iter()
                .any(|d| d.message.contains("result")),
            "a @tool result is consumer-governed, not a local sink"
        );
    }

    #[test]
    fn imported_tool_result_carries_the_tools_reads() {
        // DR-0030 X2 (cross-package baseline): an imported @tool reads a confidential
        // store and returns it; the consumer's agent may call that tool (DR-0025
        // `tools [Fetcher]`) and writes the turn result to a public outbox. The tool's
        // confidential read flows out via the turn result, so the egress must be
        // flagged — even though the consumer rule reads nothing confidential directly.
        // Folding the imported tool IR is what closes it.
        let tool = compile_program(
            r#"@tool
workflow Fetcher {
  input request Req
  output result R
  class Req { id string }
  class R { data string }
  file store secret { root "./secret"  allow read ["**"] }
  rule fetch
    when Req as request
  => {
    read text from secret at "in.txt" as loaded
    after loaded succeeds as v {
      complete result { data v.content }
    }
  }
}
"#,
        )
        .ir
        .expect("tool compiles");
        let consumer = compile_program(
            r#"@service
workflow Consumer

output result R2
class R2 { ok bool }
class Req { id string  status "open" }

agent worker { provider fixture  profile "p"  capacity 1  tools [Fetcher] }
file store outbox { root "./outbox"  allow write ["**"] }

table seed as Req [ { id "T1"  status "open" } ]

rule use
  when Req as request where request.status == "open"
  when worker is available
=> {
  tell worker as turn "go"
  after turn succeeds as outcome {
    write text to outbox at "out.txt" {
      body "x"
      mode replace
    } as written
    complete result { ok true }
  }
}
"#,
        )
        .ir
        .expect("consumer compiles");
        let envelope = Envelope::from_dsl(
            "grant file_store secret -> file:/srv/secret readable by Operator\n\
             grant file_store outbox -> file:/srv/out readable by public\n\
             grant provider fixture -> selfhost:llama readable by Operator\n",
        )
        .expect("valid");
        let verified = VerifiedEnvelope::for_test(envelope);
        // With the tool folded, the secret -> outbox leak via the turn result is caught.
        assert!(
            check_with_envelope_imports(&consumer, &verified, std::slice::from_ref(&tool))
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("secret")),
            "the imported tool's confidential read should flow out via the turn result: {:?}",
            check_with_envelope_imports(&consumer, &verified, std::slice::from_ref(&tool))
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
        // Without the import, the tool result is untracked — the gap this closes.
        assert!(!check_with_envelope_imports(&consumer, &verified, &[])
            .iter()
            .any(|d| d.message.contains("secret")));
    }

    #[test]
    fn result_dependency_reads_drops_inputs_the_result_is_independent_of() {
        // DR-0030 X2 Direction A (reach refinement): the tool reads `secret` in a side
        // rule whose recorded fact NO completing rule consumes, and reads `public_in`
        // in the rule that completes. The result provably does not depend on `secret`
        // (it never reaches a `complete`), so the refinement drops it — the result
        // carries only `public_in`, a strictly smaller join than the whole-tool box.
        let tool = compile_program(
            r#"@tool
workflow Refiner {
  input request Req
  output result R
  class Req { id string }
  class R { data string }
  class Logged { note string }
  file store secret { root "./secret"  allow read ["**"] }
  file store public_in { root "./pin"  allow read ["**"] }

  rule audit
    when Req as request
  => {
    read text from secret at "s.txt" as s
    after s succeeds as sv {
      record Logged { note sv.content }
    }
  }

  rule produce
    when Req as request
  => {
    read text from public_in at "p.txt" as p
    after p succeeds as pv {
      complete result { data pv.content }
    }
  }
}
"#,
        )
        .ir
        .expect("tool compiles");
        // the whole-tool baseline sees BOTH reads.
        let all = program_read_resources(&tool);
        assert!(all.contains(&"secret".to_owned()) && all.contains(&"public_in".to_owned()));
        // the reach refinement keeps only what the result depends on.
        let deps = result_dependency_reads(&tool);
        assert!(
            deps.contains(&"public_in".to_owned()),
            "the completing rule's read must be kept: {deps:?}"
        );
        assert!(
            !deps.contains(&"secret".to_owned()),
            "a read the result is independent of must be dropped: {deps:?}"
        );
    }

    #[test]
    fn result_field_dependency_reads_splits_reads_per_field() {
        // DR-0030 X2 v2 (per-field signature): the completing rule `combine` consumes
        // two facts via `when`, one produced from a confidential read (`secret`) and
        // one from a public read (`pub_in`). It has NO own reads. Each result field
        // references exactly one fact binding directly, so per-field reach attributes
        // `secret` to `hot` only and `pub_in` to `cold` only — a real refinement over
        // the whole-result reach (which carries both to every field).
        let tool = compile_program(
            r#"@tool
workflow Splitter {
  input request Req
  output result R
  class Req { id string }
  class Secret { s string }
  class Pub { p string }
  class R { hot string  cold string }
  file store secret { root "./sec"  allow read ["**"] }
  file store pub_in { root "./pin"  allow read ["**"] }

  rule load_secret
    when Req as request
  => {
    read text from secret at "s.txt" as s
    after s succeeds as sv {
      record Secret { s sv.content }
    }
  }

  rule load_pub
    when Req as request
  => {
    read text from pub_in at "p.txt" as p
    after p succeeds as pv {
      record Pub { p pv.content }
    }
  }

  rule combine
    when Secret as sec
    when Pub as pb
  => {
    complete result {
      hot sec.s
      cold pb.p
    }
  }
}
"#,
        )
        .ir
        .expect("tool compiles");
        let sig = result_field_dependency_reads(&tool);
        let field = |name: &str| -> Vec<String> {
            sig.iter()
                .find(|(binding, field, _)| binding == "result" && field == name)
                .map(|(_, _, reads)| reads.clone())
                .unwrap_or_else(|| panic!("no signature for result.{name}: {sig:?}"))
        };
        let hot = field("hot");
        let cold = field("cold");
        assert!(
            hot.contains(&"secret".to_owned()) && !hot.contains(&"pub_in".to_owned()),
            "hot depends on the confidential fact only: {hot:?}"
        );
        assert!(
            cold.contains(&"pub_in".to_owned()) && !cold.contains(&"secret".to_owned()),
            "cold depends on the public fact only: {cold:?}"
        );
    }

    #[test]
    fn result_field_dependency_reads_keeps_own_reads_on_every_field() {
        // The rule-level opaque box (I-IFC2) is preserved: the completing rule reads
        // `secret` DIRECTLY, so `secret` reaches EVERY result field — even `cold`,
        // whose only referenced binding is a public fact. And `hot` references a
        // within-rule DERIVED binding (`after … as sv`), whose opaque provenance falls
        // back to the whole-result reach. Neither field ever under-reports.
        let tool = compile_program(
            r#"@tool
workflow Mixer {
  input request Req
  output result R
  class Req { id string }
  class Pub { p string }
  class R { hot string  cold string }
  file store secret { root "./sec"  allow read ["**"] }
  file store pub_in { root "./pin"  allow read ["**"] }

  rule load_pub
    when Req as request
  => {
    read text from pub_in at "p.txt" as p
    after p succeeds as pv {
      record Pub { p pv.content }
    }
  }

  rule combine
    when Pub as pb
  => {
    read text from secret at "s.txt" as s
    after s succeeds as sv {
      complete result {
        cold pb.p
        hot sv.content
      }
    }
  }
}
"#,
        )
        .ir
        .expect("tool compiles");
        let sig = result_field_dependency_reads(&tool);
        let field = |name: &str| -> Vec<String> {
            sig.iter()
                .find(|(binding, field, _)| binding == "result" && field == name)
                .map(|(_, _, reads)| reads.clone())
                .unwrap_or_else(|| panic!("no signature for result.{name}: {sig:?}"))
        };
        assert!(
            field("cold").contains(&"secret".to_owned()),
            "the completing rule's own read reaches every field (opaque box): {:?}",
            field("cold")
        );
        let hot = field("hot");
        assert!(
            hot.contains(&"secret".to_owned()) && hot.contains(&"pub_in".to_owned()),
            "a derived-binding field falls back to the whole-result reach: {hot:?}"
        );
    }

    #[test]
    fn leak_and_inject_diagnostics_carry_self_serve_and_escalate_routes() {
        // a leak (read confidential ledger -> write public outbox) and an inject (an
        // unvouched source -> a high-integrity sink) each carry BOTH a self-serve route
        // (no grant) and an escalate route (a governance grant), so the whip author
        // knows what they can fix alone vs what needs the governance root agent.
        let envelope = Envelope::from_dsl(
            "grant file_store ledger -> file:/srv/ledger.db readable by Operator\n",
        )
        .expect("valid");
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        let leak = diagnostics
            .iter()
            .find(|d| d.message.contains("information-flow violation"))
            .expect("a leak should be flagged");
        let suggestion = leak.suggestion.as_deref().unwrap_or_default();
        assert!(
            suggestion.contains("self-serve") && suggestion.contains("escalate"),
            "leak fix should name both routes: {suggestion}"
        );
        assert!(
            suggestion.contains("grant declassify"),
            "the escalate route should name the declassify grant: {suggestion}"
        );
    }

    #[test]
    fn report_surfaces_invariants_violations_and_risks() {
        // envelope governs ledger (confidential) but NOT outbox, which the whip writes.
        let envelope =
            Envelope::from_json(r#"{ "resources": { "ledger": { "confidential": true } } }"#)
                .expect("valid envelope");
        let ir = ir_with_grants(&format!("{READ_LEDGER}{WRITE_OUTBOX}"));
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        // ledger gets a per-resource guaranteed invariant, not a generic line.
        assert!(
            report
                .invariants
                .iter()
                .any(|inv| inv.starts_with("ledger:")),
            "ledger should have a per-resource invariant: {:?}",
            report.invariants
        );
        // ledger (confidential) flows to outbox (not confidential) -> caught by the
        // fail-closed sticky boundary even though outbox is ungoverned.
        assert!(report.violations >= 1);
        // outbox is touched (written) but ungoverned -> flagged as a risk to confirm.
        assert!(
            report
                .flagged_risks
                .iter()
                .any(|risk| risk.starts_with("outbox:")),
            "outbox should be a flagged risk: {:?}",
            report.flagged_risks
        );
        let text = report.render();
        assert!(text.contains("guaranteed invariants"));
        assert!(text.contains("flagged risks"));
    }

    #[test]
    fn report_exposes_the_per_field_flow_signature() {
        // DR-0030 X2 v2: a producer's guarantee report surfaces the per-field flow
        // signature — the reads a consumer of each result field inherits. `hot`
        // depends on the confidential store, `cold` on the public one.
        let ir = compile_program(
            r#"@tool
workflow Splitter {
  input request Req
  output result R
  class Req { id string }
  class Secret { s string }
  class Pub { p string }
  class R { hot string  cold string }
  file store secret { root "./sec"  allow read ["**"] }
  file store pub_in { root "./pin"  allow read ["**"] }

  rule load_secret
    when Req as request
  => {
    read text from secret at "s.txt" as s
    after s succeeds as sv {
      record Secret { s sv.content }
    }
  }

  rule load_pub
    when Req as request
  => {
    read text from pub_in at "p.txt" as p
    after p succeeds as pv {
      record Pub { p pv.content }
    }
  }

  rule combine
    when Secret as sec
    when Pub as pb
  => {
    complete result {
      hot sec.s
      cold pb.p
    }
  }
}
"#,
        )
        .ir
        .expect("tool compiles");
        let envelope = Envelope::from_json(r#"{ "resources": {} }"#).expect("valid envelope");
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            report
                .flow_signature
                .iter()
                .any(|line| line.contains("result.hot") && line.contains("secret")),
            "flow signature should attribute secret to hot: {:?}",
            report.flow_signature
        );
        assert!(
            report
                .flow_signature
                .iter()
                .any(|line| line.contains("result.cold") && line.contains("pub_in")),
            "flow signature should attribute pub_in to cold: {:?}",
            report.flow_signature
        );
        assert!(report.render().contains("result/milestone flow signature"));
    }

    #[test]
    fn report_exposes_milestone_per_field_flow_signature() {
        // D3′: milestone payloads carry the same fact-granular per-field provenance
        // as `complete result`; `hot` depends on secret, `cold` on public.
        let ir = compile_program(
            r#"@tool
workflow MilestoneProducer {
  input request Req
  output result R
  class Req { id string }
  class Secret { s string }
  class Pub { p string }
  class R { ok bool }
  class Progress { hot string  cold string }
  file store secret { root "./sec"  allow read ["**"] }
  file store pub_in { root "./pin"  allow read ["**"] }

  rule load_secret
    when Req as request
  => {
    read text from secret at "s.txt" as s
    after s succeeds as sv {
      record Secret { s sv.content }
    }
  }

  rule load_pub
    when Req as request
  => {
    read text from pub_in at "p.txt" as p
    after p succeeds as pv {
      record Pub { p pv.content }
    }
  }

  rule progress
    when Secret as sec
    when Pub as pb
  => {
    emit milestone "halfway" of Progress {
      hot sec.s
      cold pb.p
    }
    complete result { ok true }
  }
}
"#,
        )
        .ir
        .expect("tool compiles");
        let envelope = Envelope::from_json(r#"{ "resources": {} }"#).expect("valid envelope");
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            report
                .flow_signature
                .iter()
                .any(|line| line.contains("milestone:halfway.hot") && line.contains("secret")),
            "flow signature should attribute secret to milestone hot: {:?}",
            report.flow_signature
        );
        assert!(
            report
                .flow_signature
                .iter()
                .any(|line| line.contains("milestone:halfway.cold") && line.contains("pub_in")),
            "flow signature should attribute pub_in to milestone cold: {:?}",
            report.flow_signature
        );
    }

    #[test]
    fn milestone_egress_is_checked_as_a_sink() {
        let ir = compile_program(
            r#"@service
workflow MilestoneLeak {
  output result R
  class R { ok bool }
  class Req { id string }
  class Progress { hot string }
  file store secret { root "./sec"  allow read ["**"] }

  table seed as Req [ { id "T1" } ]

  rule progress
    when Req as request
  => {
    read text from secret at "s.txt" as s
    after s succeeds as sv {
      emit milestone "halfway" of Progress { hot sv.content }
      complete result { ok true }
    }
  }
}
"#,
        )
        .ir
        .expect("workflow compiles");
        let envelope =
            Envelope::from_json(r#"{ "resources": { "secret": { "confidential": true } } }"#)
                .expect("valid envelope");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("secret")
                    && d.message.contains("milestone:halfway")),
            "confidential read should not flow to uncleared milestone: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
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

    /// A whip whose rule is triggered by a signal and writes the signal's payload
    /// into `ledger`. The signal `deploy.finished` carries a `status` field.
    fn signal_triggered_write_ir() -> IrProgram {
        let program = r##"@service
workflow IfcSignal

output result R
class R { ok bool }

signal deploy.finished { status string }
file store ledger { root "./ledger"  allow write ["**"] }

rule ingest
  when deploy.finished as deployed
=> {
  write text to ledger at "notes.txt" {
    body "{{ deployed.status }}"
    mode append
  } as noted
  after noted succeeds {
    complete result { ok true }
  }
}
"##;
        compile_program(program).ir.expect("compiles")
    }

    #[test]
    fn signal_trigger_is_a_low_integrity_source() {
        // H8: a rule triggered by `when <Signal> as e` reads an externally-injected
        // signal (an operator/peer `whip signal`). It defaults to public integrity
        // (fail-closed), so driving an Operator-integrity store is an injection —
        // exactly as an inbound channel message is. Before H8 the signal was
        // recognized as NO source, so this flow slipped past a governed envelope.
        let ir = signal_triggered_write_ir();
        let envelope =
            Envelope::from_dsl("grant file_store ledger -> file:/srv/ledger.db from Operator\n")
                .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("integrity violation")
                    && d.message.contains("signal:deploy.finished")
                    && d.message.contains("ledger")),
            "an untrusted signal driving a trusted sink should be an injection, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn vouched_signal_does_not_inject() {
        // a signal the envelope vouches (`signal:<name> from Operator`) carries
        // Operator integrity, so it meets the sink's requirement — no injection. The
        // integrity is envelope-declared, not kind-hardcoded (the H8 premise).
        let ir = signal_triggered_write_ir();
        let envelope = Envelope::from_dsl(
            "grant file_store ledger -> file:/srv/ledger.db from Operator\n\
             grant signal deploy.finished -> signal:deploy.finished from Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("integrity violation")),
            "a vouched signal should not inject, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    fn invoke_selector_write_ir() -> IrProgram {
        let program = r##"@service
workflow Child

input request Req
output result R
class Req { mode "drain" | "noop" }
class R { ok bool }

file store ledger { root "./ledger"  allow write ["**"] }

rule dispatch
  when Req as request
=> {
  case request.mode {
    "drain" => {
      write text to ledger at "notes.txt" {
        body "drain"
        mode append
      } as noted
      after noted succeeds {
        complete result { ok true }
      }
    }
    "noop" => {
      complete result { ok true }
    }
  }
}
"##;
        compile_program(program).ir.expect("compiles")
    }

    #[test]
    fn invoke_input_selector_cannot_gate_a_higher_integrity_sink() {
        // D2b: a workflow input is caller-controlled. Without a vouched
        // `invoke:<workflow>` port, a case on that input may not select a branch
        // that drives an Operator-integrity sink.
        let ir = invoke_selector_write_ir();
        let envelope =
            Envelope::from_dsl("grant file_store ledger -> file:/srv/ledger.db from Operator\n")
                .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("NMIF-on-invoke-selector")
                    && d.message.contains("request.mode")
                    && d.message.contains("ledger")),
            "unvouched invoke input selector should not gate ledger: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn vouched_invoke_input_selector_may_gate_matching_integrity_sink() {
        let ir = invoke_selector_write_ir();
        let envelope = Envelope::from_dsl(
            "grant file_store ledger -> file:/srv/ledger.db from Operator\n\
             grant invoke Child -> invoke:Child from Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("NMIF-on-invoke-selector")),
            "vouched invoke input should meet ledger integrity: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn confidential_signal_leaks_to_public_sink() {
        // the confidentiality axis is symmetric: a signal the envelope labels
        // readable-by Operator, written into a public store, leaks (the signal is a
        // read source on both axes).
        let ir = signal_triggered_write_ir();
        let envelope = Envelope::from_dsl(
            "grant signal deploy.finished -> signal:deploy.finished readable by Operator\n\
             grant file_store ledger -> file:/srv/ledger.db public\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("information-flow violation")
                    && d.message.contains("signal:deploy.finished")),
            "a confidential signal written to a public sink should leak, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn signal_trigger_is_in_the_ifc_surface() {
        // H8: the workflow's surface (X1) enumerates `signal:<name>` so a consumer's
        // governance must cover it (no ungoverned door).
        let ir = signal_triggered_write_ir();
        let surface = ifc_surface(&ir);
        assert!(
            surface.iter().any(|d| d == "signal:deploy.finished"),
            "surface should include the signal door, got: {surface:?}"
        );
    }

    /// A producer rule reads `source` and emits `work.done`; a consumer rule reacts
    /// `when work.done` and writes the payload into `ledger`. The carried integrity
    /// of `work.done` is the producer's read-source integrity.
    fn signal_carriage_ir() -> IrProgram {
        let program = r##"@service
workflow Carriage

output result R
class R { ok bool }
class Ticket { id string  status "open" }

signal work.done { detail string }
file store source { root "./source"  allow read ["**"] }
file store ledger { root "./ledger"  allow write ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule produce
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from source at "in.txt" as loaded
  after loaded succeeds as file {
    emit signal work.done to ticket.id {
      detail file.content
    } as sent
  }
}

rule consume
  when work.done as evt
=> {
  write text to ledger at "out.txt" {
    body "{{ evt.detail }}"
    mode append
  } as noted
  after noted succeeds {
    complete result { ok true }
  }
}
"##;
        compile_program(program).ir.expect("compiles")
    }

    #[test]
    fn internal_signal_carries_emitter_integrity() {
        // H8 stage b (THE win): `work.done` is marked internal and emitted by a rule
        // whose only read source (`source`) has Operator integrity. So the signal
        // CARRIES Operator integrity to its receiver, which writes the Operator
        // `ledger` — no injection, with no hand-vouching of the signal itself.
        let ir = signal_carriage_ir();
        let envelope = Envelope::from_dsl(
            "grant file_store source -> file:/srv/source from Operator\n\
             grant file_store ledger -> file:/srv/ledger from Operator\n\
             grant signal work.done -> signal:work.done internal\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("integrity violation")),
            "an internal signal from a trusted emitter should carry that trust, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn signal_without_internal_mark_stays_low_and_injects() {
        // the contrast: the SAME flow without the `internal` mark — `work.done` is an
        // external-entry signal (stage a), defaults low, and injects into the Operator
        // ledger. This is exactly what the `internal` mark + carriage clears above.
        let ir = signal_carriage_ir();
        let envelope = Envelope::from_dsl(
            "grant file_store source -> file:/srv/source from Operator\n\
             grant file_store ledger -> file:/srv/ledger from Operator\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("integrity violation")
                    && d.message.contains("signal:work.done")
                    && d.message.contains("ledger")),
            "an unmarked signal should default low and inject, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn internal_signal_does_not_fabricate_trust() {
        // carriage does NOT launder trust up: when the emitter's read source is
        // untrusted (no `from` → public integrity), the internal signal carries
        // `public` to the receiver, which still injects into the Operator ledger.
        let ir = signal_carriage_ir();
        let envelope = Envelope::from_dsl(
            "grant file_store source -> file:/srv/source readable by Anyone\n\
             grant file_store ledger -> file:/srv/ledger from Operator\n\
             grant signal work.done -> signal:work.done internal\n",
        )
        .expect("valid");
        let diagnostics = check_with_envelope(&ir, &VerifiedEnvelope::for_test(envelope));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("integrity violation")
                    && d.message.contains("signal:work.done")),
            "an internal signal from an untrusted emitter must still inject, got: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn cross_package_signal_carries_imported_emitter_integrity() {
        // DR-0029 / H8 stage b: a consumer reacts `when work.done` and writes the
        // Operator `ledger`; the signal is emitted by an IMPORTED tool whose emit
        // reads an Operator-integrity store. The consumer derives the imported emit's
        // carried integrity UNDER ITS OWN ENVELOPE, so `work.done` carries Operator
        // across the package boundary — no injection, no producer label attestation.
        let consumer = compile_program(
            r##"@service
workflow Consumer

output result R
class R { ok bool }

signal work.done { detail string }
file store ledger { root "./ledger"  allow write ["**"] }

rule consume
  when work.done as evt
=> {
  write text to ledger at "out.txt" {
    body "{{ evt.detail }}"
    mode append
  } as noted
  after noted succeeds {
    complete result { ok true }
  }
}
"##,
        )
        .ir
        .expect("consumer compiles");
        let imported = compile_program(
            r##"@service
workflow Producer

output result R
class R { ok bool }
class Ticket { id string  status "open" }

signal work.done { detail string }
file store source { root "./source"  allow read ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule produce
  when Ticket as ticket where ticket.status == "open"
=> {
  read text from source at "in.txt" as loaded
  after loaded succeeds as file {
    emit signal work.done to ticket.id {
      detail file.content
    } as sent
  }
}
"##,
        )
        .ir
        .expect("producer compiles");
        let envelope = Envelope::from_dsl(
            "grant file_store source -> file:/srv/source from Operator\n\
             grant file_store ledger -> file:/srv/ledger from Operator\n\
             grant signal work.done -> signal:work.done internal\n",
        )
        .expect("valid");
        let verified = VerifiedEnvelope::for_test(envelope);
        // WITHOUT the import, the consumer sees no emitter -> falls back to the
        // external-entry low -> injects into the Operator ledger.
        assert!(
            check_with_envelope(&consumer, &verified)
                .iter()
                .any(|d| d.message.contains("integrity violation")),
            "with no imported emitter the internal signal defaults low and injects"
        );
        // WITH the imported tool, the consumer derives `work.done`'s integrity from
        // the imported Operator-trusted emit -> no injection (cross-package carriage).
        assert!(
            !check_with_envelope_imports(&consumer, &verified, &[imported])
                .iter()
                .any(|d| d.message.contains("integrity violation")),
            "the imported tool's Operator-trusted emit should carry across the package boundary"
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
        let program = r##"
@service
workflow IfcSurface {
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
        invoke Child { ticket ticket } as child
        record Note { id "n1" }
      }
    }
  }
}

workflow Child {
  input ticket Ticket
  class Ticket { id string  status "open" }
}
"##;
        let compiled = compile_program_with_root(program, Some("IfcSurface"));
        let ir = compiled.ir.unwrap_or_else(|| {
            panic!(
                "compiles, diagnostics: {:?}",
                compiled
                    .diagnostics
                    .iter()
                    .map(|d| &d.message)
                    .collect::<Vec<_>>()
            )
        });
        let surface = ifc_surface(&ir);
        for expected in ["crm", "out", "fact:Note", "invoke:Child"] {
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
        assert_eq!(env.reader_label("a"), "Operator");
        assert_eq!(env.reader_label("b"), "Operator");
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
        // secret — it must not appear among the guaranteed-invariant resources (H5).
        let envelope = Envelope::from_dsl(
            "grant file_store crm -> file:/srv/crm readable by Operator\n\
             grant provider fixture -> selfhost:llama readable by Operator\n",
        )
        .expect("valid");
        let ir = ir_with_grants(READ_LEDGER);
        let report = governance_report(&ir, &VerifiedEnvelope::for_test(envelope));
        // the report names the canonical kind:address identity, not the handle (E5).
        assert!(
            report
                .invariants
                .iter()
                .any(|inv| inv.starts_with("file:/srv/crm:")),
            "crm should have a per-resource invariant under its address: {:?}",
            report.invariants
        );
        assert!(
            !report
                .invariants
                .iter()
                .any(|inv| inv.starts_with("selfhost:llama:")),
            "a provider principal must not be listed as protected data: {:?}",
            report.invariants
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
                assert!(ok.contains("guaranteed invariants"));
            }
            _ => panic!("genuine signed envelope should verify"),
        }
        // ...but tampering with the labels makes the boundary REJECT it, so neither
        // the checker nor the report can vouch for content they cannot attest.
        let tampered = json.replace("\"reader\":[\"Operator\"]", "\"reader\":[]");
        assert_ne!(tampered, json, "test should actually modify the content");
        match VerifiedEnvelope::from_text(&tampered) {
            EnvelopeStatus::Rejected(_) => {}
            _ => panic!("tampered signed envelope must be rejected"),
        }
    }
}
