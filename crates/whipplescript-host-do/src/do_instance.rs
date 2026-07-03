//! The durable-object binding of the instance step machine (DR-0033 chunk 5c).
//!
//! `DoInstanceDriver` is the DO counterpart to the native `NativeInstanceDriver`:
//! it implements the kernel's [`InstanceDriver`] seam over one held
//! `RuntimeKernel<DoSqliteStore<Sql>>`, so the same [`InstanceStepMachine`] drives
//! a workflow instance on the durable object. Because `DoSqliteStore` now
//! implements all three store traits (chunk 5a), the whole rule pass
//! (`step_instance_generic`) runs over the DO's one SQLite.
//!
//! What is wired: the rule pass (`advance_rules`), ready-effect discovery
//! (`next_ready_effect`), and `run_effect` dispatch of the lifted store-only
//! handler cores over the DO store — `event.emit`, `loft.claim`, `human.ask`, the
//! `queue.*` family (via `WorkItems`), the lease/ledger/counter coordination family
//! (via `Coordination`), and the `file.*` family (via the `FileStore` seam). The
//! HTTP effects (coerce/agent) will suspend with `EffectStep::NeedsHttp` and be
//! fulfilled through the isolate's `fetch`; that + the remaining coupled cores
//! (notify/capability) are the rest of chunk 5b, so an unlifted kind still errors
//! clearly rather than silently skipping.

use whipplescript_kernel::coerce::{CoerceRequest, CoerceResult, CoerceStatus};
use whipplescript_kernel::coerce_native::{
    build_coerce_call_parts, build_request, parse_response, CoerceCall, CoerceProvider,
};
use whipplescript_kernel::effect_config::EffectConfig;
use whipplescript_kernel::effect_handlers::{
    run_capability_effect_generic, run_coordination_effect_generic, run_event_effect_generic,
    run_file_effect_generic, run_file_import_effect_generic, run_file_write_effect_generic,
    run_human_effect_generic, run_loft_effect_generic, run_notify_effect_generic,
    run_queue_effect_generic, CapabilityContract, DeliveryGovernance,
};
use whipplescript_kernel::instance_machine::{EffectStep, InstanceDriver};
use whipplescript_kernel::rule_lowering::json_from_str;
use whipplescript_kernel::rule_pass::step_instance_generic;
use whipplescript_kernel::sansio::{HttpResponse, TransportError};
use whipplescript_kernel::{idempotency_key, CoerceExecution, RuntimeKernel};
use whipplescript_parser::IrProgram;
use whipplescript_store::files::FileStore;
use whipplescript_store::{ClaimableEffect, RunStart, RuntimeStore, StoreError};

/// Projected coerce provider credentials (the DO secrets plane supplies these; a
/// live worker reads them from its bindings). Everything else the coerce HTTP
/// effect needs is host-neutral in the kernel — `build_coerce_call_parts` +
/// `build_request` + `parse_response` + `settle_coerce_result` — so this config is
/// the whole of what the DO adds.
pub struct CoerceProviderConfig {
    pub provider: CoerceProvider,
    /// The provider name recorded on runs/terminals.
    pub provider_name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

use crate::do_store::{DoSql, DoSqliteStore};

/// Drives a workflow instance's rule pass + effect discovery on the durable object.
pub struct DoInstanceDriver<'a, Sql: DoSql> {
    /// One held kernel over the DO's SQLite (backs runtime + coordination +
    /// work-items surfaces).
    pub kernel: RuntimeKernel<DoSqliteStore<Sql>>,
    /// The DO's file byte store (small files inline in DO SQLite, large spilled) —
    /// the `FileStore` seam the file effects cross. `DoFileStore` / `TieredFileStore`.
    pub files: &'a dyn FileStore,
    /// Projected coerce provider credentials, or `None` if coerce is not configured
    /// on this DO (a `coerce.call` then errors rather than degrading silently).
    pub coerce: Option<&'a CoerceProviderConfig>,
    pub ir: &'a IrProgram,
    pub instance_id: &'a str,
}

/// Durable-object delivery governance. The mock DO is ungoverned (no envelope in
/// env), so no cross-package internal-workflow delivery is forbidden; a live DO
/// answers this from its bindings/secrets — the governance plane plugged in with
/// the infra (mirrors the native `IfcDeliveryGovernance`).
struct DoDeliveryGovernance;
impl DeliveryGovernance for DoDeliveryGovernance {
    fn any_internal_workflow(&self, _resources: &[String]) -> Result<bool, String> {
        Ok(false)
    }
}

/// Durable-object capability contract. The mock DO carries no package-lock, so no
/// output-validation constraint applies; a live DO validates against the contract
/// in its program metadata (plugged in with the infra; mirrors the native
/// `PackageLockCapabilityContract`).
struct DoCapabilityContract;
impl CapabilityContract for DoCapabilityContract {
    fn validate_output(
        &self,
        _effect: &ClaimableEffect,
        _value: &serde_json::Value,
    ) -> Option<String> {
        None
    }
}

impl<Sql: DoSql> InstanceDriver for DoInstanceDriver<'_, Sql> {
    fn advance_rules(&mut self) -> Result<bool, StoreError> {
        step_instance_generic(&mut self.kernel, self.instance_id, self.ir, None, None)?;
        let terminal = self
            .kernel
            .store()
            .status(self.instance_id)?
            .map(|status| status.instance.status != "running")
            .unwrap_or(true);
        Ok(terminal)
    }

    fn next_ready_effect(&mut self) -> Result<Option<ClaimableEffect>, StoreError> {
        Ok(self
            .kernel
            .claimable_effects(self.instance_id)?
            .into_iter()
            .next())
    }

    fn run_effect(
        &mut self,
        effect: &ClaimableEffect,
        incoming: Option<Result<HttpResponse, TransportError>>,
    ) -> Result<EffectStep, StoreError> {
        // The store-only handler cores are host-agnostic (`kernel::effect_handlers`),
        // so the DO runs them over its `RuntimeKernel<DoSqliteStore>`. Fixture
        // outcomes do not apply on the DO (real execution), so `outcome_failed` is
        // false. The rest of the store-only cores + the HTTP effects land as they
        // are lifted (chunk 5b); an unlifted kind errors clearly rather than skips.
        let config = EffectConfig {
            provider: "do".to_owned(),
            outcome_failed: false,
        };
        let event = match effect.kind.as_str() {
            "event.emit" => {
                run_event_effect_generic(&mut self.kernel, self.instance_id, effect, &config)?
            }
            "loft.claim" => {
                run_loft_effect_generic(&mut self.kernel, self.instance_id, effect, &config)?
            }
            "human.ask" => {
                run_human_effect_generic(&mut self.kernel, self.instance_id, effect, &config)?
            }
            "event.notify" => run_notify_effect_generic(
                &mut self.kernel,
                self.instance_id,
                effect,
                &DoDeliveryGovernance,
            )?,
            "capability.call" => run_capability_effect_generic(
                &mut self.kernel,
                self.instance_id,
                effect,
                &config,
                &DoCapabilityContract,
            )?,
            // The coerce HTTP effect: the sans-IO suspend/resume DR-0033 exists for.
            // First pass builds the provider request (pure kernel: parts + request)
            // and yields `NeedsHttp`; the host awaits `fetch` and re-enters with the
            // response, which `parse_response` + `settle_coerce_result` turn into the
            // terminal — every piece host-neutral in the kernel but the creds.
            "coerce" => {
                let cfg = self.coerce.ok_or_else(|| {
                    StoreError::Conflict(
                        "coerce provider is not configured on this durable object".to_owned(),
                    )
                })?;
                let input = json_from_str(&effect.input_json);
                let function_name = input
                    .get("function_name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("coerce")
                    .to_owned();
                let arguments = input
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let output_type = input
                    .get("output_type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("json")
                    .to_owned();
                let (prompt, output_schema, wrapped, schema_name) =
                    build_coerce_call_parts(self.ir, &function_name, &arguments)
                        .map_err(StoreError::Conflict)?;
                let run_id = idempotency_key(&[self.instance_id, &effect.effect_id, "coerce-run"]);
                let lease_id =
                    idempotency_key(&[self.instance_id, &effect.effect_id, "coerce-lease"]);
                let request = CoerceRequest {
                    function_name,
                    arguments_json: arguments.to_string(),
                    output_type,
                    generated_coerce_source_hash: "do".to_owned(),
                    input_schema_hash: "do".to_owned(),
                    output_schema_hash: "do".to_owned(),
                };
                match incoming {
                    // Prepare: build the provider request and suspend on `fetch`.
                    None => {
                        self.kernel.start_run(RunStart {
                            instance_id: self.instance_id,
                            effect_id: &effect.effect_id,
                            run_id: &run_id,
                            provider: &cfg.provider_name,
                            worker_id: "whip-worker",
                            lease_id: &lease_id,
                            lease_expires_at: "2030-01-01T00:00:00Z",
                            metadata_json: "{}",
                        })?;
                        let call = CoerceCall {
                            provider: cfg.provider,
                            base_url: &cfg.base_url,
                            api_key: &cfg.api_key,
                            model: &cfg.model,
                            prompt: &prompt,
                            output_schema: &output_schema,
                            schema_name: &schema_name,
                            max_tokens: cfg.max_tokens,
                            codex: None,
                        };
                        return Ok(EffectStep::NeedsHttp(build_request(&call)));
                    }
                    // Finish: decode the fetched response (or a transport failure)
                    // and settle it through the shared kernel seam.
                    resumed => {
                        let result = match resumed {
                            Some(Ok(response)) => parse_response(cfg.provider, &response, wrapped),
                            other => CoerceResult {
                                status: CoerceStatus::Failed,
                                value_json: None,
                                error_json: Some(
                                    serde_json::json!({
                                        "transport": format!("{other:?}"),
                                    })
                                    .to_string(),
                                ),
                                summary: "coerce transport error".to_owned(),
                                transcript: String::new(),
                                usage_json: r#"{"input_tokens":0,"output_tokens":0}"#.to_owned(),
                            },
                        };
                        let execution = CoerceExecution {
                            instance_id: self.instance_id,
                            effect_id: &effect.effect_id,
                            run_id: &run_id,
                            provider: &cfg.provider_name,
                            worker_id: "whip-worker",
                            lease_id: &lease_id,
                            lease_expires_at: "2030-01-01T00:00:00Z",
                            request: &request,
                        };
                        self.kernel.settle_coerce_result(execution, &result)?
                    }
                }
            }
            "queue.file" | "queue.claim" | "queue.release" | "queue.finish" => {
                run_queue_effect_generic(&mut self.kernel, self.instance_id, effect, &config)?
            }
            "lease.acquire" | "lease.release" | "ledger.append" | "counter.consume" => {
                run_coordination_effect_generic(&mut self.kernel, self.instance_id, effect)?
            }
            "file.read" => {
                run_file_effect_generic(&mut self.kernel, self.files, self.instance_id, effect)?
            }
            "file.write" => run_file_write_effect_generic(
                &mut self.kernel,
                self.files,
                self.instance_id,
                effect,
            )?,
            "file.import" => run_file_import_effect_generic(
                &mut self.kernel,
                self.files,
                self.instance_id,
                effect,
            )?,
            other => {
                return Err(StoreError::Conflict(format!(
                    "effect kind `{other}` is not yet executable on the durable object \
                     (its handler core is not lifted / HTTP wiring pending — chunk 5b)"
                )))
            }
        };
        Ok(EffectStep::Done(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whipplescript_kernel::instance_machine::{InstanceOutcome, InstanceStepMachine};
    use whipplescript_kernel::sansio::{run_to_completion, HostDriver, IoRequest, IoResult};
    use whipplescript_kernel::ProgramVersionInput;
    use whipplescript_store::NewInstanceAuthority;

    use crate::do_store::test_support::store;

    /// Refuses I/O — a store-only / effect-free run never asks for it.
    struct RefuseIoHost;
    impl HostDriver for RefuseIoHost {
        fn fulfill(&self, _request: &IoRequest) -> IoResult {
            IoResult::Http(Err(TransportError::Transport(
                "no DO I/O expected".to_owned(),
            )))
        }
    }

    /// A `FileStore` stub for effect-free runs (no file effect touches it).
    struct NoFiles;
    impl FileStore for NoFiles {
        fn read_to_string(&self, _path: &std::path::Path) -> std::io::Result<String> {
            Err(std::io::Error::other("no files in this test"))
        }
        fn exists(&self, _path: &std::path::Path) -> bool {
            false
        }
        fn create_dir_all(&self, _path: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
        fn write(&self, _path: &std::path::Path, _bytes: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("no files in this test"))
        }
        fn append(&self, _path: &std::path::Path, _bytes: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("no files in this test"))
        }
    }

    // The DO drives an effect-free workflow's rule pass to its terminal through the
    // InstanceStepMachine, over `RuntimeKernel<DoSqliteStore>` — proving the whole
    // instance scheduler runs on the durable-object store.
    #[test]
    fn do_instance_driver_drives_rule_pass_to_terminal() {
        // The smallest complete workflow (examples/minimal-noop.whip): observe
        // start, record a fact, finish. Effect-free, so it drives to a terminal
        // purely through the rule pass. Compile it, then create + start an instance
        // directly in the DO SQLite via the kernel.
        let source = "workflow MinimalNoop\n\noutput result StartupSeen\n\n\
             class StartupSeen {\n  source string\n  state \"observed\"\n}\n\n\
             rule observe_start\n  when started\n=> {\n\
             \x20 record StartupSeen {\n    source \"external.started\"\n    state \"observed\"\n  }\n\n\
             \x20 complete result {\n    source \"external.started\"\n    state \"observed\"\n  }\n}\n";
        let compiled = whipplescript_parser::compile_program(source);
        let ir = compiled.ir.expect("program compiles");

        let mut kernel = RuntimeKernel::new(store());
        let version = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &ir.workflow,
                    source_hash: "src",
                    ir_hash: "ir",
                    compiler_version: "test",
                },
                &ir,
            )
            .expect("program version");
        let instance_id = kernel
            .create_instance_with_authority(
                &version,
                "{}",
                NewInstanceAuthority {
                    workflow_principal: "local/MinimalNoop",
                    effective_authority_json: "{}",
                },
            )
            .expect("instance");
        // Seed the `when started` trigger (the `external.started` event; the
        // rule fires on it directly, no input fact needed).
        kernel
            .ingest_external_event(&instance_id, "external.started", "{}", Some("started"))
            .expect("start event");

        let driver = DoInstanceDriver {
            kernel,
            files: &NoFiles,
            coerce: None,
            ir: &ir,
            instance_id: &instance_id,
        };
        let mut machine = InstanceStepMachine::new(driver);
        let outcome = run_to_completion(&mut machine, &RefuseIoHost);
        assert!(
            matches!(outcome, InstanceOutcome::Terminal),
            "the DO drives the instance to a terminal: {outcome:?}"
        );

        let driver = machine.into_driver();
        let status = driver
            .kernel
            .store()
            .status(&instance_id)
            .expect("status")
            .expect("instance row");
        assert_eq!(status.instance.status, "completed");
    }

    /// A host that answers the coerce `fetch` with a canned Anthropic structured
    /// output (mirrors `coerce_native`'s parse fixtures).
    struct CoerceHost;
    impl HostDriver for CoerceHost {
        fn fulfill(&self, request: &IoRequest) -> IoResult {
            let IoRequest::Http(_) = request;
            IoResult::Http(Ok(HttpResponse {
                status: 200,
                body: serde_json::json!({
                    "content": [
                        { "type": "tool_use", "name": "Decision", "input": { "score": 0.9 } }
                    ],
                    "usage": { "input_tokens": 1, "output_tokens": 1 }
                }),
            }))
        }
    }

    // The architectural crux (DR-0033): a coerce HTTP effect SUSPENDS the instance
    // on `fetch` (EffectStep::NeedsHttp -> Outcome::NeedsIo) and RESUMES to its
    // terminal when the host supplies the response — all on the durable-object store.
    #[test]
    fn do_instance_driver_suspends_and_settles_a_coerce_effect() {
        let source = "workflow CoerceScore\n\noutput result Decision\n\n\
             class Decision {\n  score float\n}\n\n\
             coerce scoreIt() -> Decision {\n  prompt \"\"\"\n  Score it.\n  {{ ctx.output_format }}\n  \"\"\"\n}\n\n\
             rule go\n  when started\n=> {\n  coerce scoreIt() as review\n\
             \x20 after review succeeds as decision {\n    complete result { score decision.score }\n  }\n\
             \x20 after review fails {\n    complete result { score 0.0 }\n  }\n}\n";
        let ir = whipplescript_parser::compile_program(source)
            .ir
            .expect("coerce program compiles");
        // Register the builtin coerce provider + capability (migration-0001 seeds a
        // live store carries; the minimal test store does not).
        let store = store();
        for stmt in [
            "INSERT INTO capability_schemas (capability, description, schema_json) \
             VALUES ('coerce', 'Coerce unstructured data into a typed value.', '{}')",
            "INSERT INTO effect_providers (provider_id, effect_kind, provider, capability, config_json) \
             VALUES ('provider_coerce_builtin', 'coerce', 'builtin-coerce', 'coerce', '{}')",
            "INSERT INTO capability_bindings (binding_id, program_id, capability, provider, config_json) \
             VALUES ('binding_coerce_builtin', NULL, 'coerce', 'builtin-coerce', '{}')",
        ] {
            store.sql.execute(stmt, &[]).expect("seed coerce provider");
        }
        let mut kernel = RuntimeKernel::new(store);
        let version = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &ir.workflow,
                    source_hash: "src",
                    ir_hash: "ir",
                    compiler_version: "test",
                },
                &ir,
            )
            .expect("program version");
        let instance_id = kernel
            .create_instance_with_authority(
                &version,
                "{}",
                NewInstanceAuthority {
                    workflow_principal: "local/CoerceScore",
                    effective_authority_json: "{}",
                },
            )
            .expect("instance");
        kernel
            .ingest_external_event(&instance_id, "external.started", "{}", Some("started"))
            .expect("start event");

        let cfg = CoerceProviderConfig {
            provider: CoerceProvider::Anthropic,
            provider_name: "anthropic".to_owned(),
            base_url: "https://api.anthropic.com".to_owned(),
            api_key: "test-key".to_owned(),
            model: "claude-test".to_owned(),
            max_tokens: 1024,
        };
        let driver = DoInstanceDriver {
            kernel,
            files: &NoFiles,
            coerce: Some(&cfg),
            ir: &ir,
            instance_id: &instance_id,
        };
        let mut machine = InstanceStepMachine::new(driver);
        let outcome = run_to_completion(&mut machine, &CoerceHost);
        let driver = machine.into_driver();
        assert!(
            matches!(outcome, InstanceOutcome::Terminal),
            "the DO suspends on fetch and settles the coerce to a terminal: {outcome:?}"
        );

        let status = driver
            .kernel
            .store()
            .status(&instance_id)
            .expect("status")
            .expect("instance row");
        assert_eq!(status.instance.status, "completed");
    }
}
