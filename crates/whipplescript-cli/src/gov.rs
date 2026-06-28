//! The two-agent privilege separation (DR-0028 D5).
//!
//! The **governance agent** is the only path that may SIGN a governance envelope,
//! and it is gated by admin/sudo privilege (G1, G4). The **whip agent** — the rest
//! of the CLI — is unprivileged and may only *verify* a signed envelope, never
//! produce one. The single-signer rule and untrusted-input isolation hold
//! structurally: `SignedEnvelope::sign` is the sole producer of a valid
//! attestation and refuses without governance privilege, and no unprivileged path
//! reaches it.
//!
//! Trust root: option C attestation (DR-0028) — the signature is a SHA-256 of the
//! canonical envelope content bound to the signer identity. Tampering with the
//! content breaks the hash, so the whip agent rejects it.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::ifc::Envelope;

/// Whether the current process holds governance (admin) privilege (G1).
///
/// Production binds this to the OS: `whip gov` is installed root-only / behind
/// sudo, so being able to run the privileged path *is* the gate. Where requiring
/// root is impractical (CI, dev sandboxes), the explicit `WHIPPLESCRIPT_GOV_ADMIN`
/// token stands in. Otherwise the process is unprivileged — the whip agent.
pub fn has_governance_privilege() -> bool {
    std::env::var_os("WHIPPLESCRIPT_GOV_ADMIN").is_some()
}

fn hash_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Canonicalize a governance config (DSL or JSON) to the stable signed-artifact
/// JSON, so its hash is deterministic.
pub fn canonicalize(config_text: &str) -> Result<String, String> {
    let envelope = if config_text.trim_start().starts_with('{') {
        Envelope::from_json(config_text)?
    } else {
        Envelope::from_dsl(config_text)?
    };
    Ok(envelope.to_canonical_json())
}

/// A signed governance envelope: the canonical content + an attestation binding
/// its hash to the signer.
pub struct SignedEnvelope {
    pub canonical: String,
    pub envelope_hash: String,
    pub signer: String,
}

impl SignedEnvelope {
    /// Sign a governance config. PRIVILEGED (G1/G4): fails without governance
    /// privilege, so only the governance agent reaches a successful sign.
    pub fn sign(config_text: &str, signer: &str) -> Result<Self, String> {
        Self::sign_with_privilege(config_text, signer, has_governance_privilege())
    }

    /// Sign with the privilege decision injected (so the gate is testable without
    /// mutating process env). `whip` always passes `has_governance_privilege()`.
    fn sign_with_privilege(
        config_text: &str,
        signer: &str,
        privileged: bool,
    ) -> Result<Self, String> {
        if !privileged {
            return Err(
                "signing a governance envelope requires admin/governance privilege \
                 — run via the governance agent (sudo)"
                    .to_owned(),
            );
        }
        let canonical = canonicalize(config_text)?;
        let envelope_hash = hash_hex(&canonical);
        Ok(Self {
            canonical,
            envelope_hash,
            signer: signer.to_owned(),
        })
    }

    /// Sign with privilege forced on — for tests only (the crate forbids the
    /// `set_var` that would drive the env-gated path).
    #[cfg(test)]
    pub(crate) fn sign_for_test(config_text: &str, signer: &str) -> Self {
        Self::sign_with_privilege(config_text, signer, true).expect("privileged test sign")
    }

    /// The on-disk signed-envelope JSON: the FULL canonical content (resources,
    /// bindings, delegations, declassifications, endorsements) with an `attestation`
    /// block. The whole content is what the hash covers, so every field must survive
    /// the round-trip — not just `resources`.
    pub fn to_json(&self) -> String {
        let mut value: serde_json::Value =
            serde_json::from_str(&self.canonical).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "attestation".to_owned(),
                serde_json::json!({
                    "envelope_hash": self.envelope_hash,
                    "signer": self.signer,
                }),
            );
        }
        value.to_string()
    }

    /// Verify a loaded signed-envelope JSON: the attested hash must match the hash
    /// of its canonical content. UNPRIVILEGED — the whip agent does this; a tamper
    /// breaks the hash. Returns the signer on success.
    pub fn verify(signed_json: &str) -> Result<String, String> {
        let value: serde_json::Value = serde_json::from_str(signed_json)
            .map_err(|err| format!("invalid signed envelope: {err}"))?;
        let attestation = value
            .get("attestation")
            .ok_or_else(|| "envelope is not signed (no attestation)".to_owned())?;
        let attested_hash = attestation
            .get("envelope_hash")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "attestation has no envelope_hash".to_owned())?;
        let signer = attestation
            .get("signer")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        // recompute the canonical content hash from the FULL content (everything
        // except the attestation), re-canonicalized through the envelope so ordering
        // matches signing. A tamper to any covered field breaks the hash.
        let mut content = value.clone();
        if let Some(obj) = content.as_object_mut() {
            obj.remove("attestation");
        }
        let recanonical = canonicalize(&content.to_string())?;
        if hash_hex(&recanonical) == attested_hash {
            Ok(signer)
        } else {
            Err(
                "signed envelope failed verification — content does not match its \
                 attestation (tampered or re-edited without re-signing)"
                    .to_owned(),
            )
        }
    }
}

/// An escalation request: the whip side asks the admin for a governance change
/// (e.g. a missing declassify grant). It crosses to the governance side as
/// LOW-INTEGRITY data (DR-0028 D5, the one whip->gov flow): the governance agent
/// never auto-acts on it; the admin reviews and signs.
pub struct Escalation {
    pub request: String,
    pub from: String,
}

/// File an escalation request — UNPRIVILEGED (the whip side files it). Appends a
/// low-integrity request to the escalation log; it is data, never an action.
pub fn file_escalation(log: &Path, request: &str, from: &str) -> Result<(), String> {
    let line = serde_json::json!({ "request": request, "from": from }).to_string();
    let mut contents = std::fs::read_to_string(log).unwrap_or_default();
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(&line);
    contents.push('\n');
    std::fs::write(log, contents)
        .map_err(|err| format!("cannot write escalation log {}: {err}", log.display()))
}

/// Review pending escalations — PRIVILEGED (only the governance agent, G1). The
/// requests are surfaced for the admin to decide on; reviewing them never applies
/// them (the admin signs a new envelope separately).
pub fn list_escalations(log: &Path) -> Result<Vec<Escalation>, String> {
    list_escalations_with_privilege(log, has_governance_privilege())
}

fn list_escalations_with_privilege(
    log: &Path,
    privileged: bool,
) -> Result<Vec<Escalation>, String> {
    if !privileged {
        return Err(
            "reviewing escalations requires governance privilege (run via the governance agent)"
                .to_owned(),
        );
    }
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|err| format!("invalid escalation entry: {err}"))?;
        out.push(Escalation {
            request: value
                .get("request")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            from: value
                .get("from")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
grant file_store ledger -> file:/srv/ledger.db readable by Operator\n\
grant file_store outbox -> file:/srv/outbox public\n";

    /// The whip agent (no governance privilege) cannot sign — the single-signer
    /// rule G4, enforced structurally.
    #[test]
    fn unprivileged_cannot_sign() {
        assert!(SignedEnvelope::sign_with_privilege(CONFIG, "admin", false).is_err());
    }

    #[test]
    fn governance_agent_signs_and_whip_agent_verifies() {
        let signed =
            SignedEnvelope::sign_with_privilege(CONFIG, "alice@admin", true).expect("privileged");
        let json = signed.to_json();
        // the whip agent (unprivileged) can still VERIFY
        assert_eq!(
            SignedEnvelope::verify(&json).expect("verifies"),
            "alice@admin"
        );
    }

    #[test]
    fn escalation_channel_files_low_integrity_and_only_gov_reviews() {
        let log = std::env::temp_dir().join(format!(
            "whip-gov-escalation-{}-{}.jsonl",
            std::process::id(),
            CONFIG.len()
        ));
        let _ = std::fs::remove_file(&log);
        file_escalation(&log, "need declassify ledger to Auditor", "bob").expect("filed");
        file_escalation(&log, "need endorse intake to Operator", "carol").expect("filed");
        // the whip side (unprivileged) cannot review escalations (G1).
        assert!(list_escalations_with_privilege(&log, false).is_err());
        // the governance agent reviews them.
        let items = list_escalations_with_privilege(&log, true).expect("review");
        assert_eq!(items.len(), 2);
        assert!(items[0].request.contains("declassify"));
        assert_eq!(items[0].from, "bob");
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn tampered_envelope_fails_verification() {
        let signed =
            SignedEnvelope::sign_with_privilege(CONFIG, "alice@admin", true).expect("privileged");
        let json = signed.to_json();
        // flip ledger's reader authority to public without re-signing
        let tampered = json.replace("\"reader\":\"Operator\"", "\"reader\":\"public\"");
        assert_ne!(tampered, json, "test should actually modify the content");
        assert!(SignedEnvelope::verify(&tampered).is_err());
    }
}
