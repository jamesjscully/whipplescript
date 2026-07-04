//! Shared types for the WhippleScript rule-machine runtime.

use std::collections::{BTreeMap, BTreeSet};

/// Current implementation stage for the active redesign.
pub const IMPLEMENTATION_STAGE: &str = "stage-0-skeleton";

/// Returns the workspace package version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compile-time library and effect-contract registry.
///
/// This is the concrete data boundary between source-level package/library
/// meaning and runtime provider execution. It is intentionally plain data:
/// compiler and CLI surfaces can expose it without giving package code any
/// parser or runtime authority.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContractRegistry {
    pub libraries: Vec<LibraryRegistration>,
    pub constructs: Vec<ConstructRegistration>,
    pub effect_contracts: Vec<EffectContract>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryRegistration {
    pub id: String,
    pub version: String,
    pub standard: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstructRegistration {
    pub id: String,
    pub library_id: String,
    pub version: String,
    pub construct_family: String,
    pub keyword: String,
    pub scope: String,
    pub fields: Vec<ConstructField>,
    pub requires: Vec<ConstructInterface>,
    pub provides: Vec<ConstructInterface>,
    pub lowering_target: String,
    pub target_capability: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstructField {
    pub name: String,
    pub kind: String,
    pub required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstructInterface {
    pub kind: String,
    pub name: Option<String>,
    pub type_ref: Option<String>,
    pub phase: String,
    pub cardinality: String,
}

pub const CORE_CAPABILITY_CALL_CONSTRUCT_ID: &str = "core.capability.call";
pub const CONSTRUCT_FAMILY_DECLARATION_BLOCK: &str = "declaration_block";
pub const CONSTRUCT_FAMILY_EFFECT_OPERATION: &str = "effect_operation";
pub const CONSTRUCT_FAMILY_EFFECT_CONTRACT: &str = "effect_contract";
pub const CONSTRUCT_FAMILY_SOURCE_DECLARATION: &str = "source_declaration";
pub const CONSTRUCT_FAMILY_ASSERTION: &str = "assertion";
pub const CONSTRUCT_FAMILY_RULE: &str = "rule";
pub const CONSTRUCT_FAMILY_PROJECTION_READ: &str = "projection_read";
pub const CONSTRUCT_LOWERING_METADATA: &str = "metadata";
pub const CONSTRUCT_LOWERING_METADATA_ONLY: &str = "metadata_only";
pub const CONSTRUCT_LOWERING_CAPABILITY_CALL: &str = "capability_call";
pub const CONSTRUCT_LOWERING_TYPED_EFFECT_CALL: &str = "typed_effect_call";
pub const CONSTRUCT_LOWERING_RESOURCE_EFFECT: &str = "resource_effect";
pub const CONSTRUCT_LOWERING_CORE_EFFECT: &str = "core_effect";
pub const CONSTRUCT_LOWERING_SIGNAL_EMIT: &str = "signal_emit";
pub const CONSTRUCT_LOWERING_SIGNAL_SOURCE: &str = "signal_source";
pub const CONSTRUCT_LOWERING_CLOCK_SOURCE: &str = "clock_source";
pub const CONSTRUCT_LOWERING_SCHEDULE_EMITTER: &str = "schedule_emitter";
pub const CONSTRUCT_LOWERING_RULE_TEMPLATE: &str = "rule_template";
pub const CONSTRUCT_LOWERING_PROJECTION_VIEW: &str = "projection_view";
pub const CONSTRUCT_LOWERING_ASSERTION_CHECK: &str = "assertion_check";
pub const CONSTRUCT_SCOPE_RULE_BODY: &str = "rule_body";
pub const CONSTRUCT_INTERFACE_CAPABILITY: &str = "Capability";
pub const CONSTRUCT_INTERFACE_EFFECT_HANDLE: &str = "EffectHandle";
pub const CONSTRUCT_INTERFACE_PHASE_COMPILE_RUNTIME: &str = "compile/runtime";
pub const CONSTRUCT_INTERFACE_CARDINALITY_EXACTLY_ONE: &str = "exactly-one";

pub const CONSTRUCT_STATIC_DETERMINISTIC: &str = "deterministic";
pub const CONSTRUCT_STATIC_CONTRACT_PINNED: &str = "contract_pinned";
pub const CONSTRUCT_STATIC_NO_RUNTIME_INPUTS: &str = "no_runtime_inputs";
pub const CONSTRUCT_STATIC_NO_HIDDEN_AUTHORITY: &str = "no_hidden_authority";
pub const CONSTRUCT_STATIC_NO_PACKAGE_SCHEDULER: &str = "no_package_scheduler";
pub const CONSTRUCT_STATIC_NO_PACKAGE_LIFECYCLE: &str = "no_package_lifecycle";
pub const CONSTRUCT_STATIC_NO_DIRECT_FACT_WRITE: &str = "no_direct_fact_write";
pub const CONSTRUCT_STATIC_NO_DIRECT_RULE_FIRE: &str = "no_direct_rule_fire";

pub const CONSTRUCT_PLATFORM_STATIC_GUARANTEES: &[&str] = &[
    CONSTRUCT_STATIC_DETERMINISTIC,
    CONSTRUCT_STATIC_CONTRACT_PINNED,
    CONSTRUCT_STATIC_NO_RUNTIME_INPUTS,
    CONSTRUCT_STATIC_NO_HIDDEN_AUTHORITY,
    CONSTRUCT_STATIC_NO_PACKAGE_SCHEDULER,
    CONSTRUCT_STATIC_NO_PACKAGE_LIFECYCLE,
    CONSTRUCT_STATIC_NO_DIRECT_FACT_WRITE,
    CONSTRUCT_STATIC_NO_DIRECT_RULE_FIRE,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructTargetCapabilityPolicy {
    Forbidden,
    RequiredCapabilityCallContract,
}

impl ConstructTargetCapabilityPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::RequiredCapabilityCallContract => "required_capability_call_contract",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructLoweringAuthorityProfile {
    None,
    CapabilityScoped,
    EventAdmission,
    ProjectionSource,
}

impl ConstructLoweringAuthorityProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::CapabilityScoped => "capability_scoped",
            Self::EventAdmission => "event_admission",
            Self::ProjectionSource => "projection_source",
        }
    }
}

/// The single diagnostic severity scale, aligned 1:1 with the LSP severities
/// (spec/error-handling.md). This is the canonical set for every diagnostic-
/// producing surface (check, lint, LSP, test). `note` is NOT a severity — it is
/// related information attached to a diagnostic. Inbox-item / notification
/// "severity" is a distinct concept and is not this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    /// Every severity, highest to lowest, for exhaustive iteration.
    pub const ALL: [Severity; 4] = [Self::Error, Self::Warning, Self::Info, Self::Hint];

    /// The stable wire/serialized token for this severity.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        }
    }

    /// Parse a wire token back into a severity. Returns `None` for any token
    /// outside the canonical set (e.g. the unrelated inbox `"normal"`).
    pub fn from_wire(value: &str) -> Option<Severity> {
        match value {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "info" => Some(Self::Info),
            "hint" => Some(Self::Hint),
            _ => None,
        }
    }

    /// The Language Server Protocol `DiagnosticSeverity` number for this severity.
    /// The canonical set aligns 1:1 with LSP (Error=1, Warning=2, Info=3, Hint=4),
    /// so editor tooling can map a diagnostic severity without a lookup table.
    pub fn lsp_code(self) -> i32 {
        match self {
            Self::Error => 1,
            Self::Warning => 2,
            Self::Info => 3,
            Self::Hint => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformConstructFamily {
    pub id: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformConstructLowering {
    pub id: &'static str,
    pub compatible_families: &'static [&'static str],
    pub package_authorable: bool,
    pub required_scope: Option<&'static str>,
    pub target_capability: ConstructTargetCapabilityPolicy,
    pub required_interfaces: &'static [&'static str],
    pub provided_interfaces: &'static [&'static str],
    pub lifecycle_profiles: &'static [&'static str],
    pub authority_profile: ConstructLoweringAuthorityProfile,
    pub static_guarantees: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformReservedKeywordPrivilege {
    pub keyword: &'static str,
    pub library_id: &'static str,
    pub construct_family: &'static str,
    pub scope: &'static str,
    pub lowering_target: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformConstructCatalog {
    pub families: &'static [PlatformConstructFamily],
    pub lowerings: &'static [PlatformConstructLowering],
    pub scopes: &'static [&'static str],
    pub field_kinds: &'static [&'static str],
    pub interface_kinds: &'static [&'static str],
    pub interface_phases: &'static [&'static str],
    pub interface_cardinalities: &'static [&'static str],
    pub reserved_keywords: &'static [&'static str],
    pub reserved_keyword_privileges: &'static [PlatformReservedKeywordPrivilege],
}

impl PlatformConstructCatalog {
    pub fn family(&self, id: &str) -> Option<&'static PlatformConstructFamily> {
        self.families.iter().find(|family| family.id == id)
    }

    pub fn lowering(&self, id: &str) -> Option<&'static PlatformConstructLowering> {
        self.lowerings.iter().find(|lowering| lowering.id == id)
    }

    pub fn family_ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.families.iter().map(|family| family.id)
    }

    pub fn lowering_ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.lowerings.iter().map(|lowering| lowering.id)
    }

    pub fn lowerings_for_family<'a>(
        &'a self,
        family: &'a str,
    ) -> impl Iterator<Item = &'a PlatformConstructLowering> + 'a {
        self.lowerings
            .iter()
            .filter(move |lowering| lowering.compatible_families.contains(&family))
    }

    pub fn contains_scope(&self, scope: &str) -> bool {
        self.scopes.contains(&scope)
    }

    pub fn contains_field_kind(&self, kind: &str) -> bool {
        self.field_kinds.contains(&kind)
    }

    pub fn contains_interface_kind(&self, kind: &str) -> bool {
        self.interface_kinds.contains(&kind)
    }

    pub fn contains_interface_phase(&self, phase: &str) -> bool {
        self.interface_phases.contains(&phase)
    }

    pub fn contains_interface_cardinality(&self, cardinality: &str) -> bool {
        self.interface_cardinalities.contains(&cardinality)
    }

    pub fn contains_reserved_keyword(&self, keyword: &str) -> bool {
        self.reserved_keywords.contains(&keyword)
    }

    pub fn reserved_keyword_privilege(
        &self,
        library_id: &str,
        keyword: &str,
        construct_family: &str,
        scope: &str,
        lowering_target: &str,
    ) -> Option<&'static PlatformReservedKeywordPrivilege> {
        self.reserved_keyword_privileges.iter().find(|privilege| {
            privilege.library_id == library_id
                && privilege.keyword == keyword
                && privilege.construct_family == construct_family
                && privilege.scope == scope
                && privilege.lowering_target == lowering_target
        })
    }
}

pub const PLATFORM_CONSTRUCT_CATALOG: PlatformConstructCatalog = PlatformConstructCatalog {
    families: &[
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_DECLARATION_BLOCK,
            description: "package-declared block syntax that lowers to metadata",
        },
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_EFFECT_OPERATION,
            description: "rule-body operation syntax that lowers to a core effect template",
        },
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_EFFECT_CONTRACT,
            description: "package effect-contract metadata used by capability resolution",
        },
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_SOURCE_DECLARATION,
            description: "top-level source blocks that lower to signal-source and clock-source admission templates",
        },
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_ASSERTION,
            description: "assertions that lower to assertion checks",
        },
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_RULE,
            description: "rules that lower to rule templates and fact writes",
        },
        PlatformConstructFamily {
            id: CONSTRUCT_FAMILY_PROJECTION_READ,
            description: "checker-owned projection reads used by rules and assertions",
        },
    ],
    lowerings: &[
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_METADATA,
            compatible_families: &[
                CONSTRUCT_FAMILY_EFFECT_CONTRACT,
                CONSTRUCT_FAMILY_PROJECTION_READ,
            ],
            package_authorable: false,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[],
            lifecycle_profiles: &["none"],
            authority_profile: ConstructLoweringAuthorityProfile::None,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_METADATA_ONLY,
            compatible_families: &[CONSTRUCT_FAMILY_DECLARATION_BLOCK],
            package_authorable: true,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[],
            lifecycle_profiles: &["none"],
            authority_profile: ConstructLoweringAuthorityProfile::None,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_CAPABILITY_CALL,
            compatible_families: &[CONSTRUCT_FAMILY_EFFECT_OPERATION],
            package_authorable: true,
            required_scope: Some(CONSTRUCT_SCOPE_RULE_BODY),
            target_capability: ConstructTargetCapabilityPolicy::RequiredCapabilityCallContract,
            required_interfaces: &[CONSTRUCT_INTERFACE_CAPABILITY],
            provided_interfaces: &[CONSTRUCT_INTERFACE_EFFECT_HANDLE],
            lifecycle_profiles: &["effect_graph", "typed_effect_graph"],
            authority_profile: ConstructLoweringAuthorityProfile::CapabilityScoped,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_TYPED_EFFECT_CALL,
            compatible_families: &[CONSTRUCT_FAMILY_EFFECT_OPERATION],
            // Promoted to package-authorable for `std.files` (DR-0019 / DR-0020
            // chain): read/write/import/export lower through `typed_effect_call`
            // (`requires Capability<…>` + typed output, `target_capability`
            // Forbidden — distinct from `capability_call`).
            package_authorable: true,
            required_scope: Some(CONSTRUCT_SCOPE_RULE_BODY),
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[CONSTRUCT_INTERFACE_CAPABILITY],
            provided_interfaces: &[CONSTRUCT_INTERFACE_EFFECT_HANDLE],
            lifecycle_profiles: &["typed_effect_graph"],
            authority_profile: ConstructLoweringAuthorityProfile::CapabilityScoped,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_RESOURCE_EFFECT,
            compatible_families: &[CONSTRUCT_FAMILY_EFFECT_OPERATION],
            package_authorable: false,
            required_scope: Some(CONSTRUCT_SCOPE_RULE_BODY),
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &["Resource"],
            provided_interfaces: &[CONSTRUCT_INTERFACE_EFFECT_HANDLE],
            lifecycle_profiles: &["resource_effect_graph"],
            authority_profile: ConstructLoweringAuthorityProfile::CapabilityScoped,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_CORE_EFFECT,
            compatible_families: &[CONSTRUCT_FAMILY_EFFECT_OPERATION],
            package_authorable: false,
            required_scope: Some(CONSTRUCT_SCOPE_RULE_BODY),
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[CONSTRUCT_INTERFACE_EFFECT_HANDLE],
            lifecycle_profiles: &["effect_graph", "typed_effect_graph"],
            authority_profile: ConstructLoweringAuthorityProfile::CapabilityScoped,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_SIGNAL_EMIT,
            compatible_families: &[CONSTRUCT_FAMILY_EFFECT_OPERATION],
            package_authorable: false,
            required_scope: Some(CONSTRUCT_SCOPE_RULE_BODY),
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &["Event"],
            provided_interfaces: &[],
            lifecycle_profiles: &["event_record"],
            authority_profile: ConstructLoweringAuthorityProfile::EventAdmission,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_SIGNAL_SOURCE,
            compatible_families: &[CONSTRUCT_FAMILY_SOURCE_DECLARATION],
            package_authorable: false,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[],
            lifecycle_profiles: &["signal_source_template"],
            authority_profile: ConstructLoweringAuthorityProfile::EventAdmission,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_CLOCK_SOURCE,
            compatible_families: &[CONSTRUCT_FAMILY_SOURCE_DECLARATION],
            package_authorable: false,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[],
            lifecycle_profiles: &["clock_source_template"],
            authority_profile: ConstructLoweringAuthorityProfile::EventAdmission,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_SCHEDULE_EMITTER,
            compatible_families: &[CONSTRUCT_FAMILY_EFFECT_OPERATION],
            package_authorable: false,
            required_scope: Some(CONSTRUCT_SCOPE_RULE_BODY),
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[CONSTRUCT_INTERFACE_EFFECT_HANDLE],
            lifecycle_profiles: &["schedule_template"],
            authority_profile: ConstructLoweringAuthorityProfile::EventAdmission,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_RULE_TEMPLATE,
            compatible_families: &[CONSTRUCT_FAMILY_RULE],
            package_authorable: false,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[],
            lifecycle_profiles: &["rule_template"],
            authority_profile: ConstructLoweringAuthorityProfile::None,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_PROJECTION_VIEW,
            compatible_families: &[CONSTRUCT_FAMILY_PROJECTION_READ],
            package_authorable: false,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &["Projection"],
            provided_interfaces: &[],
            lifecycle_profiles: &["event_projection"],
            authority_profile: ConstructLoweringAuthorityProfile::ProjectionSource,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
        PlatformConstructLowering {
            id: CONSTRUCT_LOWERING_ASSERTION_CHECK,
            compatible_families: &[CONSTRUCT_FAMILY_ASSERTION],
            package_authorable: false,
            required_scope: None,
            target_capability: ConstructTargetCapabilityPolicy::Forbidden,
            required_interfaces: &[],
            provided_interfaces: &[],
            lifecycle_profiles: &["assertion_check"],
            authority_profile: ConstructLoweringAuthorityProfile::None,
            static_guarantees: CONSTRUCT_PLATFORM_STATIC_GUARANTEES,
        },
    ],
    scopes: &[
        "top_level",
        CONSTRUCT_SCOPE_RULE_BODY,
        "workflow_body",
        "expression",
    ],
    field_kinds: &[
        "identifier",
        "string",
        "number",
        "boolean",
        "duration",
        "type_ref",
        "provider_ref",
        "capability_ref",
        "event_ref",
        "effect_ref",
        "expression",
        "predicate",
        "list",
        "record",
        "enum",
    ],
    interface_kinds: &[
        "Resource",
        "Projection",
        "Event",
        "SignalSource",
        "EffectContract",
        "Operation",
        CONSTRUCT_INTERFACE_CAPABILITY,
        "ProviderKind",
        "Profile",
        "Binding",
        CONSTRUCT_INTERFACE_EFFECT_HANDLE,
        "TerminalOutput",
        "Value",
        "ContextArtifact",
        "Diagnostic",
    ],
    interface_phases: &[
        "compile",
        "runtime",
        CONSTRUCT_INTERFACE_PHASE_COMPILE_RUNTIME,
    ],
    interface_cardinalities: &[
        CONSTRUCT_INTERFACE_CARDINALITY_EXACTLY_ONE,
        "optional-one",
        "many",
        "named-many",
    ],
    reserved_keywords: &[
        "agent", "ask", "call", "cancel", "case", "claim", "class", "coerce", "complete",
        "counter", "decide", "effect", "else", "emit", "enum", "event", "fail", "flow", "from",
        "harness", "if", "ledger", "lease", "let", "match", "queue", "release", "renew", "rule",
        "signal", "tell", "then", "use", "when", "workflow",
    ],
    reserved_keyword_privileges: &[
        PlatformReservedKeywordPrivilege {
            keyword: "claim",
            library_id: "std.tracker",
            construct_family: CONSTRUCT_FAMILY_EFFECT_OPERATION,
            scope: CONSTRUCT_SCOPE_RULE_BODY,
            lowering_target: CONSTRUCT_LOWERING_CAPABILITY_CALL,
        },
        PlatformReservedKeywordPrivilege {
            keyword: "renew",
            library_id: "std.tracker",
            construct_family: CONSTRUCT_FAMILY_EFFECT_OPERATION,
            scope: CONSTRUCT_SCOPE_RULE_BODY,
            lowering_target: CONSTRUCT_LOWERING_CAPABILITY_CALL,
        },
        PlatformReservedKeywordPrivilege {
            keyword: "release",
            library_id: "std.tracker",
            construct_family: CONSTRUCT_FAMILY_EFFECT_OPERATION,
            scope: CONSTRUCT_SCOPE_RULE_BODY,
            lowering_target: CONSTRUCT_LOWERING_CAPABILITY_CALL,
        },
    ],
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectContract {
    pub id: String,
    pub library_id: String,
    pub version: String,
    pub effect_kind: String,
    pub source_forms: Vec<String>,
    pub input_schema: Option<String>,
    pub output_schema: Option<String>,
    pub required_capabilities: Vec<String>,
    pub provider_kinds: Vec<String>,
    pub projected_facts: Vec<String>,
    pub validation: TypedOutputValidation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypedOutputValidation {
    None,
    RuntimeBoundary,
}

impl TypedOutputValidation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RuntimeBoundary => "runtime_boundary",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractRegistryDiagnostic {
    pub code: String,
    pub message: String,
}

// --- Embedded standard-package manifest data (M5) -----------------------------
//
// std packages ship as data compiled into the binary rather than as scattered
// per-package builtin functions. This is the first entry (std.messaging `send`);
// the parser's `contract_registry` and the CLI both source std definitions from
// here, so there is one source of truth. Each `std_*` function returns the same
// `ConstructRegistration` / `EffectContract` the parser previously hand-rolled,
// so the move is behaviour-preserving. Later slices relocate the remaining std
// libraries here and drive `use std.*` authority from the same data.

/// The `messaging.send` capability id — the target of the `send` construct and
/// the id of its `capability.call` effect contract.
pub const MESSAGING_SEND_CAPABILITY: &str = "messaging.send";

/// `std.messaging` `send` construct registration (effect_operation → capability.call).
pub fn std_messaging_send_construct() -> ConstructRegistration {
    ConstructRegistration {
        id: MESSAGING_SEND_CAPABILITY.to_owned(),
        library_id: "std.messaging".to_owned(),
        version: "v0".to_owned(),
        construct_family: CONSTRUCT_FAMILY_EFFECT_OPERATION.to_owned(),
        keyword: "send".to_owned(),
        scope: CONSTRUCT_SCOPE_RULE_BODY.to_owned(),
        fields: vec![
            ConstructField {
                name: "channel".to_owned(),
                kind: "identifier".to_owned(),
                required: true,
            },
            ConstructField {
                name: "text".to_owned(),
                kind: "expression".to_owned(),
                required: true,
            },
        ],
        requires: vec![ConstructInterface {
            kind: CONSTRUCT_INTERFACE_CAPABILITY.to_owned(),
            name: Some(MESSAGING_SEND_CAPABILITY.to_owned()),
            type_ref: None,
            phase: CONSTRUCT_INTERFACE_PHASE_COMPILE_RUNTIME.to_owned(),
            cardinality: CONSTRUCT_INTERFACE_CARDINALITY_EXACTLY_ONE.to_owned(),
        }],
        provides: Vec::new(),
        lowering_target: CONSTRUCT_LOWERING_CAPABILITY_CALL.to_owned(),
        target_capability: Some(MESSAGING_SEND_CAPABILITY.to_owned()),
    }
}

/// `std.messaging` `messaging.send` `capability.call` effect contract — the
/// target the `send` construct lowers to.
pub fn std_messaging_send_effect_contract() -> EffectContract {
    EffectContract {
        id: MESSAGING_SEND_CAPABILITY.to_owned(),
        library_id: "std.messaging".to_owned(),
        version: "v0".to_owned(),
        effect_kind: "capability.call".to_owned(),
        source_forms: vec!["send".to_owned()],
        input_schema: Some("messaging.send.input".to_owned()),
        output_schema: Some("MessageSendReceipt".to_owned()),
        required_capabilities: vec![MESSAGING_SEND_CAPABILITY.to_owned()],
        provider_kinds: vec!["messaging".to_owned()],
        projected_facts: vec!["effect.output".to_owned()],
        validation: TypedOutputValidation::RuntimeBoundary,
    }
}

impl ContractRegistry {
    pub fn merge(&mut self, other: ContractRegistry) {
        for library in other.libraries {
            self.upsert_library(library);
        }
        for form in other.constructs {
            self.upsert_construct(form);
        }
        for contract in other.effect_contracts {
            self.upsert_effect_contract(contract);
        }
        self.libraries.sort_by(|left, right| left.id.cmp(&right.id));
        self.constructs.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });
        self.effect_contracts.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });
    }

    pub fn upsert_library(&mut self, library: LibraryRegistration) {
        if let Some(existing) = self
            .libraries
            .iter_mut()
            .find(|existing| existing.id == library.id)
        {
            if existing.version == "unlocked" && library.version != "unlocked" {
                *existing = library;
            } else if existing.version == library.version {
                existing.standard |= library.standard;
            } else if library.version != "unlocked" {
                self.libraries.push(library);
            }
            return;
        }
        self.libraries.push(library);
    }

    pub fn upsert_construct(&mut self, form: ConstructRegistration) {
        if self.constructs.iter().any(|existing| {
            existing.id == form.id && existing.version == form.version && existing == &form
        }) {
            return;
        }
        self.constructs.push(form);
    }

    pub fn upsert_effect_contract(&mut self, contract: EffectContract) {
        if let Some(existing) = self
            .effect_contracts
            .iter_mut()
            .find(|existing| existing.id == contract.id && existing.version == contract.version)
        {
            if existing.library_id == contract.library_id
                && existing.effect_kind == contract.effect_kind
                && existing.input_schema == contract.input_schema
                && existing.output_schema == contract.output_schema
                && existing.validation == contract.validation
            {
                merge_unique_list(&mut existing.source_forms, &contract.source_forms);
                merge_unique_list(
                    &mut existing.required_capabilities,
                    &contract.required_capabilities,
                );
                merge_unique_list(&mut existing.provider_kinds, &contract.provider_kinds);
                merge_unique_list(&mut existing.projected_facts, &contract.projected_facts);
            } else {
                self.effect_contracts.push(contract);
            }
            return;
        }
        self.effect_contracts.push(contract);
    }

    pub fn validate(&self) -> Vec<ContractRegistryDiagnostic> {
        let mut diagnostics = Vec::new();
        let mut libraries = BTreeSet::new();

        for library in &self.libraries {
            if library.id.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "library_id_empty",
                    "library registration has an empty id",
                ));
            }
            if library.version.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "library_version_empty",
                    format!("library `{}` has an empty version", library.id),
                ));
            }
            if !libraries.insert(library.id.clone()) {
                diagnostics.push(registry_diagnostic(
                    "library_duplicate",
                    format!("library `{}` is registered more than once", library.id),
                ));
            }
        }

        let library_ids = self
            .libraries
            .iter()
            .map(|library| library.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut constructs = BTreeSet::new();
        let mut construct_keywords = BTreeSet::new();

        for form in &self.constructs {
            if form.id.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "construct_id_empty",
                    "construct has an empty id",
                ));
            }
            if form.version.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "construct_version_empty",
                    format!("construct `{}` has an empty version", form.id),
                ));
            }
            if form.keyword.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "construct_keyword_empty",
                    format!("construct `{}` has an empty keyword", form.id),
                ));
            }
            if form.construct_family.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "construct_family_empty",
                    format!("construct `{}` has an empty construct family", form.id),
                ));
            }
            if form.scope.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "construct_scope_empty",
                    format!("construct `{}` has an empty scope", form.id),
                ));
            }
            if form.lowering_target.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "construct_lowering_target_empty",
                    format!("construct `{}` has an empty lowering target", form.id),
                ));
            }
            if form
                .target_capability
                .as_deref()
                .is_some_and(|target| target.trim().is_empty())
            {
                diagnostics.push(registry_diagnostic(
                    "construct_target_capability_empty",
                    format!("construct `{}` has an empty target capability", form.id),
                ));
            }
            if !library_ids.contains(form.library_id.as_str()) {
                diagnostics.push(registry_diagnostic(
                    "construct_unknown_library",
                    format!(
                        "construct `{}` references unknown library `{}`",
                        form.id, form.library_id
                    ),
                ));
            }
            if !constructs.insert((form.id.clone(), form.version.clone())) {
                diagnostics.push(registry_diagnostic(
                    "construct_duplicate",
                    format!(
                        "construct `{}` version `{}` is registered more than once",
                        form.id, form.version
                    ),
                ));
            }
            if !construct_keywords.insert((form.scope.clone(), form.keyword.clone())) {
                diagnostics.push(registry_diagnostic(
                    "construct_keyword_duplicate",
                    format!(
                        "construct keyword `{}` is registered more than once for `{}`",
                        form.keyword, form.scope
                    ),
                ));
            }
            validate_construct_fields(form, &mut diagnostics);
            validate_construct_interfaces(form, "requires", &form.requires, &mut diagnostics);
            validate_construct_interfaces(form, "provides", &form.provides, &mut diagnostics);
        }

        let mut contracts = BTreeSet::new();

        for contract in &self.effect_contracts {
            if contract.id.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "effect_contract_id_empty",
                    "effect contract has an empty id",
                ));
            }
            if contract.version.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "effect_contract_version_empty",
                    format!("effect contract `{}` has an empty version", contract.id),
                ));
            }
            if contract.effect_kind.trim().is_empty() {
                diagnostics.push(registry_diagnostic(
                    "effect_kind_empty",
                    format!("effect contract `{}` has an empty effect kind", contract.id),
                ));
            }
            if contract.source_forms.is_empty() {
                diagnostics.push(registry_diagnostic(
                    "source_forms_empty",
                    format!("effect contract `{}` declares no source forms", contract.id),
                ));
            }
            if !library_ids.contains(contract.library_id.as_str()) {
                diagnostics.push(registry_diagnostic(
                    "effect_contract_unknown_library",
                    format!(
                        "effect contract `{}` references unknown library `{}`",
                        contract.id, contract.library_id
                    ),
                ));
            }
            if !contracts.insert((contract.id.clone(), contract.version.clone())) {
                diagnostics.push(registry_diagnostic(
                    "effect_contract_duplicate",
                    format!(
                        "effect contract `{}` version `{}` is registered more than once",
                        contract.id, contract.version
                    ),
                ));
            }
            validate_unique_list(
                "required_capability_duplicate",
                &format!("effect contract `{}`", contract.id),
                "required capability",
                &contract.required_capabilities,
                &mut diagnostics,
            );
            validate_unique_list(
                "provider_kind_duplicate",
                &format!("effect contract `{}`", contract.id),
                "provider kind",
                &contract.provider_kinds,
                &mut diagnostics,
            );
            validate_unique_list(
                "projected_fact_duplicate",
                &format!("effect contract `{}`", contract.id),
                "projected fact",
                &contract.projected_facts,
                &mut diagnostics,
            );
            if contract.validation == TypedOutputValidation::RuntimeBoundary
                && contract.output_schema.is_none()
            {
                diagnostics.push(registry_diagnostic(
                    "runtime_validation_without_output_schema",
                    format!(
                        "effect contract `{}` uses runtime validation without an output schema",
                        contract.id
                    ),
                ));
            }
            if !contract.projected_facts.is_empty() && contract.output_schema.is_none() {
                diagnostics.push(registry_diagnostic(
                    "projection_without_output_schema",
                    format!(
                        "effect contract `{}` projects facts without an output schema",
                        contract.id
                    ),
                ));
            }
        }

        diagnostics
    }
}

fn validate_construct_fields(
    form: &ConstructRegistration,
    diagnostics: &mut Vec<ContractRegistryDiagnostic>,
) {
    let mut fields = BTreeMap::new();
    for field in &form.fields {
        if field.name.trim().is_empty() {
            diagnostics.push(registry_diagnostic(
                "construct_field_name_empty",
                format!("construct `{}` has a field with an empty name", form.id),
            ));
        }
        if field.kind.trim().is_empty() {
            diagnostics.push(registry_diagnostic(
                "construct_field_kind_empty",
                format!(
                    "construct `{}` field `{}` has an empty kind",
                    form.id, field.name
                ),
            ));
        }
        let count = fields.entry(field.name.clone()).or_insert(0usize);
        *count += 1;
    }
    for (field, count) in fields {
        if count > 1 {
            diagnostics.push(registry_diagnostic(
                "construct_field_duplicate",
                format!(
                    "construct `{}` declares field `{field}` more than once",
                    form.id
                ),
            ));
        }
    }
}

fn validate_construct_interfaces(
    form: &ConstructRegistration,
    direction: &str,
    interfaces: &[ConstructInterface],
    diagnostics: &mut Vec<ContractRegistryDiagnostic>,
) {
    for interface in interfaces {
        if interface.kind.trim().is_empty() {
            diagnostics.push(registry_diagnostic(
                "construct_interface_kind_empty",
                format!(
                    "construct `{}` {direction} interface has an empty kind",
                    form.id
                ),
            ));
        }
        if interface
            .name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            diagnostics.push(registry_diagnostic(
                "construct_interface_name_empty",
                format!(
                    "construct `{}` {direction} interface `{}` has an empty name",
                    form.id, interface.kind
                ),
            ));
        }
        if interface
            .type_ref
            .as_deref()
            .is_some_and(|type_ref| type_ref.trim().is_empty())
        {
            diagnostics.push(registry_diagnostic(
                "construct_interface_type_empty",
                format!(
                    "construct `{}` {direction} interface `{}` has an empty type",
                    form.id, interface.kind
                ),
            ));
        }
        if interface.phase.trim().is_empty() {
            diagnostics.push(registry_diagnostic(
                "construct_interface_phase_empty",
                format!(
                    "construct `{}` {direction} interface `{}` has an empty phase",
                    form.id, interface.kind
                ),
            ));
        }
        if interface.cardinality.trim().is_empty() {
            diagnostics.push(registry_diagnostic(
                "construct_interface_cardinality_empty",
                format!(
                    "construct `{}` {direction} interface `{}` has an empty cardinality",
                    form.id, interface.kind
                ),
            ));
        }
    }
}

fn merge_unique_list(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
    target.sort();
}

fn registry_diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
) -> ContractRegistryDiagnostic {
    ContractRegistryDiagnostic {
        code: code.into(),
        message: message.into(),
    }
}

fn validate_unique_list(
    code: &str,
    owner: &str,
    label: &str,
    values: &[String],
    diagnostics: &mut Vec<ContractRegistryDiagnostic>,
) {
    let mut seen = BTreeMap::new();
    for value in values {
        let count = seen.entry(value).or_insert(0usize);
        *count += 1;
    }
    for (value, count) in seen {
        if count > 1 {
            diagnostics.push(registry_diagnostic(
                code,
                format!("{owner} declares {label} `{value}` more than once"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_stage_marker() {
        assert_eq!(IMPLEMENTATION_STAGE, "stage-0-skeleton");
    }

    #[test]
    fn exposes_version() {
        assert!(!version().is_empty());
    }

    #[test]
    fn severity_round_trips_the_canonical_set() {
        assert_eq!(Severity::ALL.len(), 4);
        for severity in Severity::ALL {
            assert_eq!(Severity::from_wire(severity.as_str()), Some(severity));
        }
        assert_eq!(
            Severity::ALL.map(Severity::as_str),
            ["error", "warning", "info", "hint"]
        );
        // `note` is related-information, not a severity; inbox `normal` is unrelated.
        assert_eq!(Severity::from_wire("note"), None);
        assert_eq!(Severity::from_wire("normal"), None);
    }

    #[test]
    fn severity_lsp_codes_align_one_to_one() {
        // The canonical set maps 1:1 onto LSP DiagnosticSeverity numbers.
        assert_eq!(
            Severity::ALL.map(Severity::lsp_code),
            [1, 2, 3, 4],
            "error/warning/info/hint must map to LSP 1/2/3/4"
        );
    }

    #[test]
    fn platform_construct_catalog_defines_current_executable_slice() {
        assert!(PLATFORM_CONSTRUCT_CATALOG
            .family(CONSTRUCT_FAMILY_DECLARATION_BLOCK)
            .is_some());
        assert!(PLATFORM_CONSTRUCT_CATALOG
            .family(CONSTRUCT_FAMILY_EFFECT_OPERATION)
            .is_some());

        let capability_call = PLATFORM_CONSTRUCT_CATALOG
            .lowering(CONSTRUCT_LOWERING_CAPABILITY_CALL)
            .expect("capability_call lowering");
        assert_eq!(
            capability_call.compatible_families,
            &[CONSTRUCT_FAMILY_EFFECT_OPERATION]
        );
        assert!(capability_call.package_authorable);
        assert_eq!(
            capability_call.required_scope,
            Some(CONSTRUCT_SCOPE_RULE_BODY)
        );
        assert_eq!(
            capability_call.target_capability,
            ConstructTargetCapabilityPolicy::RequiredCapabilityCallContract
        );
        assert_eq!(
            capability_call.required_interfaces,
            &[CONSTRUCT_INTERFACE_CAPABILITY]
        );
        assert_eq!(
            capability_call.provided_interfaces,
            &[CONSTRUCT_INTERFACE_EFFECT_HANDLE]
        );
        assert_eq!(
            capability_call.lifecycle_profiles,
            &["effect_graph", "typed_effect_graph"]
        );
        assert_eq!(
            capability_call.authority_profile,
            ConstructLoweringAuthorityProfile::CapabilityScoped
        );
        assert_eq!(
            capability_call.static_guarantees,
            CONSTRUCT_PLATFORM_STATIC_GUARANTEES
        );

        let metadata_only = PLATFORM_CONSTRUCT_CATALOG
            .lowering(CONSTRUCT_LOWERING_METADATA_ONLY)
            .expect("metadata_only lowering");
        assert_eq!(
            metadata_only.compatible_families,
            &[CONSTRUCT_FAMILY_DECLARATION_BLOCK]
        );
        assert!(metadata_only.package_authorable);
        assert_eq!(
            metadata_only.target_capability,
            ConstructTargetCapabilityPolicy::Forbidden
        );

        assert!(PLATFORM_CONSTRUCT_CATALOG
            .family(CONSTRUCT_FAMILY_SOURCE_DECLARATION)
            .is_some());

        let signal_source = PLATFORM_CONSTRUCT_CATALOG
            .lowering(CONSTRUCT_LOWERING_SIGNAL_SOURCE)
            .expect("signal_source lowering");
        assert!(!signal_source.package_authorable);
        assert_eq!(
            signal_source.compatible_families,
            &[CONSTRUCT_FAMILY_SOURCE_DECLARATION]
        );
        assert_eq!(
            signal_source.lifecycle_profiles,
            &["signal_source_template"]
        );

        let clock_source = PLATFORM_CONSTRUCT_CATALOG
            .lowering(CONSTRUCT_LOWERING_CLOCK_SOURCE)
            .expect("clock_source lowering");
        assert!(!clock_source.package_authorable);
        assert_eq!(
            clock_source.compatible_families,
            &[CONSTRUCT_FAMILY_SOURCE_DECLARATION]
        );
        assert_eq!(clock_source.lifecycle_profiles, &["clock_source_template"]);
        assert_eq!(
            clock_source.authority_profile,
            ConstructLoweringAuthorityProfile::EventAdmission
        );

        assert!(PLATFORM_CONSTRUCT_CATALOG.contains_reserved_keyword("claim"));
        assert!(PLATFORM_CONSTRUCT_CATALOG.contains_reserved_keyword("lease"));
        assert!(PLATFORM_CONSTRUCT_CATALOG
            .reserved_keyword_privilege(
                "std.tracker",
                "claim",
                CONSTRUCT_FAMILY_EFFECT_OPERATION,
                CONSTRUCT_SCOPE_RULE_BODY,
                CONSTRUCT_LOWERING_CAPABILITY_CALL,
            )
            .is_some());
        assert!(PLATFORM_CONSTRUCT_CATALOG
            .reserved_keyword_privilege(
                "memory",
                "claim",
                CONSTRUCT_FAMILY_EFFECT_OPERATION,
                CONSTRUCT_SCOPE_RULE_BODY,
                CONSTRUCT_LOWERING_CAPABILITY_CALL,
            )
            .is_none());
    }

    #[test]
    fn validates_duplicate_and_malformed_contracts() {
        let registry = ContractRegistry {
            libraries: vec![LibraryRegistration {
                id: "std.coerce".to_owned(),
                version: "v0".to_owned(),
                standard: true,
            }],
            constructs: vec![
                ConstructRegistration {
                    id: "coerce.form".to_owned(),
                    library_id: "std.coerce".to_owned(),
                    version: "v0".to_owned(),
                    construct_family: "declaration_block".to_owned(),
                    keyword: "coerce".to_owned(),
                    scope: "top_level".to_owned(),
                    fields: vec![ConstructField {
                        name: "name".to_owned(),
                        kind: "identifier".to_owned(),
                        required: true,
                    }],
                    requires: Vec::new(),
                    provides: Vec::new(),
                    lowering_target: "metadata_only".to_owned(),
                    target_capability: None,
                },
                ConstructRegistration {
                    id: "coerce.form".to_owned(),
                    library_id: "missing".to_owned(),
                    version: "v0".to_owned(),
                    construct_family: String::new(),
                    keyword: "coerce".to_owned(),
                    scope: "top_level".to_owned(),
                    fields: vec![
                        ConstructField {
                            name: "name".to_owned(),
                            kind: "identifier".to_owned(),
                            required: true,
                        },
                        ConstructField {
                            name: "name".to_owned(),
                            kind: String::new(),
                            required: false,
                        },
                    ],
                    requires: vec![ConstructInterface {
                        kind: String::new(),
                        name: Some(String::new()),
                        type_ref: Some(String::new()),
                        phase: String::new(),
                        cardinality: String::new(),
                    }],
                    provides: Vec::new(),
                    lowering_target: String::new(),
                    target_capability: Some(String::new()),
                },
            ],
            effect_contracts: vec![
                EffectContract {
                    id: "coerce".to_owned(),
                    library_id: "std.coerce".to_owned(),
                    version: "v0".to_owned(),
                    effect_kind: "coerce".to_owned(),
                    source_forms: vec!["coerce".to_owned()],
                    input_schema: Some("coerce.input".to_owned()),
                    output_schema: Some("typed-provider-output".to_owned()),
                    required_capabilities: vec!["model.invoke".to_owned()],
                    provider_kinds: vec!["model".to_owned()],
                    projected_facts: vec!["effect.output".to_owned()],
                    validation: TypedOutputValidation::RuntimeBoundary,
                },
                EffectContract {
                    id: "coerce".to_owned(),
                    library_id: "missing".to_owned(),
                    version: "v0".to_owned(),
                    effect_kind: "coerce".to_owned(),
                    source_forms: Vec::new(),
                    input_schema: None,
                    output_schema: None,
                    required_capabilities: vec![
                        "model.invoke".to_owned(),
                        "model.invoke".to_owned(),
                    ],
                    provider_kinds: Vec::new(),
                    projected_facts: vec!["effect.output".to_owned()],
                    validation: TypedOutputValidation::RuntimeBoundary,
                },
            ],
        };

        let codes = registry
            .validate()
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<BTreeSet<_>>();

        assert!(codes.contains("effect_contract_unknown_library"));
        assert!(codes.contains("effect_contract_duplicate"));
        assert!(codes.contains("construct_unknown_library"));
        assert!(codes.contains("construct_duplicate"));
        assert!(codes.contains("construct_keyword_duplicate"));
        assert!(codes.contains("construct_family_empty"));
        assert!(codes.contains("construct_field_duplicate"));
        assert!(codes.contains("construct_field_kind_empty"));
        assert!(codes.contains("construct_interface_kind_empty"));
        assert!(codes.contains("construct_interface_name_empty"));
        assert!(codes.contains("construct_interface_type_empty"));
        assert!(codes.contains("construct_interface_phase_empty"));
        assert!(codes.contains("construct_interface_cardinality_empty"));
        assert!(codes.contains("construct_lowering_target_empty"));
        assert!(codes.contains("source_forms_empty"));
        assert!(codes.contains("required_capability_duplicate"));
        assert!(codes.contains("runtime_validation_without_output_schema"));
        assert!(codes.contains("projection_without_output_schema"));
    }

    #[test]
    fn merge_replaces_unlocked_import_with_locked_library() {
        let mut registry = ContractRegistry {
            libraries: vec![LibraryRegistration {
                id: "memory".to_owned(),
                version: "unlocked".to_owned(),
                standard: false,
            }],
            constructs: Vec::new(),
            effect_contracts: Vec::new(),
        };

        registry.merge(ContractRegistry {
            libraries: vec![LibraryRegistration {
                id: "memory".to_owned(),
                version: "0.1.0".to_owned(),
                standard: false,
            }],
            constructs: Vec::new(),
            effect_contracts: Vec::new(),
        });

        assert_eq!(
            registry.libraries,
            vec![LibraryRegistration {
                id: "memory".to_owned(),
                version: "0.1.0".to_owned(),
                standard: false,
            }]
        );
        assert_eq!(registry.validate(), Vec::new());
    }
}
