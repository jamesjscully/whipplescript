//! The durable-object instance handle (DR-0033 chunk 5c) — the `create` / `step` /
//! `snapshot` orchestration the TS worker shell drives.
//!
//! On a live Durable Object the isolate can only `await fetch`, so the runtime is
//! driven as a resumable step machine: JS calls [`DurableInstance::step`], and
//! either gets back a [`DurableStepOutcome::NeedsHttp`] (perform the `fetch`, call
//! `step` again with the response) or a terminal. This is exactly the
//! [`InstanceStepMachine`](whipplescript_kernel::instance_machine) fixpoint, inlined
//! here so the machine's `in_flight` effect persists in the handle across separate
//! JS calls (and thus across isolate evictions once the handle is rehydrated from
//! DO storage).
//!
//! This handle is plain Rust over any [`DoSql`] + the effect seams (files, coerce
//! creds, agent model/tools). The `#[wasm_bindgen]` surface that the live worker
//! imports is a thin wrapper over these three methods, adding only the JS glue
//! (a `DoSql` backed by `state.storage.sql`, a `fetch`-backed model client, and
//! JSON marshalling) — it carries no orchestration logic of its own.

use whipplescript_kernel::coerce_native::CoerceProvider;
use whipplescript_kernel::harness_loop::{HttpModelClient, ToolExecutor};
use whipplescript_kernel::instance_machine::{EffectStep, InstanceDriver};
use whipplescript_kernel::sansio::{HttpRequest, HttpResponse, TransportError};
use whipplescript_kernel::{ProgramVersionInput, RuntimeKernel};
use whipplescript_parser::IrProgram;
use whipplescript_store::files::FileStore;
use whipplescript_store::{ClaimableEffect, NewInstanceAuthority, RuntimeStore, StoreError};

use crate::do_instance::{CoerceProviderConfig, DoInstanceDriver};
use crate::do_store::{DoSql, DoSqliteStore};

/// What one [`DurableInstance::step`] yields back to the worker shell.
#[derive(Debug)]
pub enum DurableStepOutcome {
    /// Perform this HTTP request via `fetch` and call `step` again with the
    /// response. The in-flight effect is held in the handle until then.
    NeedsHttp(HttpRequest),
    /// The instance reached a workflow terminal (absorbing).
    Terminal,
    /// Quiescent but not terminal — parked awaiting external input / an alarm.
    Parked,
    /// A store error aborted the pass (surfaced, not swallowed).
    Failed(String),
}

/// A no-op file store for instances whose workflows touch no file effects. A live
/// worker passes a real `DoFileStore` (small files inline in DO SQLite).
struct NoFileStore;
impl FileStore for NoFileStore {
    fn read_to_string(&self, _path: &std::path::Path) -> std::io::Result<String> {
        Err(std::io::Error::other(
            "no file store configured on this instance",
        ))
    }
    fn exists(&self, _path: &std::path::Path) -> bool {
        false
    }
    fn create_dir_all(&self, _path: &std::path::Path) -> std::io::Result<()> {
        Ok(())
    }
    fn write(&self, _path: &std::path::Path, _bytes: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "no file store configured on this instance",
        ))
    }
    fn append(&self, _path: &std::path::Path, _bytes: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "no file store configured on this instance",
        ))
    }
}

/// A tool executor that errors on any request (turns declaring no tools never hit
/// it). A live worker passes one that brokers tools to an HTTP sidecar.
struct NoToolExecutor;
impl ToolExecutor for NoToolExecutor {
    fn execute(
        &self,
        call: &whipplescript_kernel::harness_loop::ToolCall,
    ) -> whipplescript_kernel::harness_loop::ToolOutcome {
        whipplescript_kernel::harness_loop::ToolOutcome {
            status: whipplescript_kernel::harness_loop::ToolStatus::Error,
            content: format!("no tool executor configured: {}", call.name),
        }
    }
}

/// The effect seams a live worker injects from its bindings/secrets. All optional
/// so an instance running only store-only + effect-free workflows needs none.
#[derive(Default)]
pub struct DurableEffectPorts {
    pub files: Option<Box<dyn FileStore>>,
    pub coerce: Option<CoerceProviderConfig>,
    pub agent_model: Option<Box<dyn HttpModelClient>>,
    pub agent_tools: Option<Box<dyn ToolExecutor>>,
}

/// A workflow instance running on the durable object as a resumable step machine.
/// Owns the kernel over the DO's SQLite, the compiled program, and the currently
/// in-flight effect (persisted across `step` calls / evictions).
pub struct DurableInstance<Sql: DoSql> {
    kernel: Option<RuntimeKernel<DoSqliteStore<Sql>>>,
    ir: IrProgram,
    instance_id: String,
    in_flight: Option<ClaimableEffect>,
    files: Box<dyn FileStore>,
    coerce: Option<CoerceProviderConfig>,
    agent_model: Option<Box<dyn HttpModelClient>>,
    agent_tools: Box<dyn ToolExecutor>,
}

impl<Sql: DoSql> DurableInstance<Sql> {
    /// Compile `program_source`, then create + start a fresh instance in the DO
    /// store with `input_json` as its input. The worker calls this once when the
    /// object is first addressed; `step` drives it thereafter.
    pub fn create(
        sql: Sql,
        program_source: &str,
        input_json: &str,
        workflow_principal: &str,
        ports: DurableEffectPorts,
    ) -> Result<Self, String> {
        let ir = whipplescript_parser::compile_program(program_source)
            .ir
            .ok_or_else(|| "program did not compile".to_owned())?;
        let mut kernel = RuntimeKernel::new(DoSqliteStore { sql });
        let version = kernel
            .create_program_version_for_program(
                ProgramVersionInput {
                    program_name: &ir.workflow,
                    source_hash: "do",
                    ir_hash: "do",
                    compiler_version: "do",
                },
                &ir,
            )
            .map_err(|error| format!("{error:?}"))?;
        let instance_id = kernel
            .create_instance_with_authority(
                &version,
                input_json,
                NewInstanceAuthority {
                    workflow_principal,
                    effective_authority_json: "{}",
                },
            )
            .map_err(|error| format!("{error:?}"))?;
        kernel
            .ingest_external_event(
                &instance_id,
                "external.started",
                input_json,
                Some("started"),
            )
            .map_err(|error| format!("{error:?}"))?;
        Ok(Self {
            kernel: Some(kernel),
            ir,
            instance_id,
            in_flight: None,
            files: ports.files.unwrap_or_else(|| Box::new(NoFileStore)),
            coerce: ports.coerce,
            agent_model: ports.agent_model,
            agent_tools: ports
                .agent_tools
                .unwrap_or_else(|| Box::new(NoToolExecutor)),
        })
    }

    /// Advance the instance until it next needs an HTTP round or settles. `incoming`
    /// is the response to the request the previous `step` returned (`None` on the
    /// first call). This is the `InstanceStepMachine` fixpoint with the in-flight
    /// effect held in `self`.
    pub fn step(
        &mut self,
        incoming: Option<Result<HttpResponse, TransportError>>,
    ) -> DurableStepOutcome {
        // Borrow disjoint fields: the driver takes the kernel by value and the effect
        // seams + program by reference, while `in_flight` is threaded separately.
        let kernel = match self.kernel.take() {
            Some(kernel) => kernel,
            None => {
                return DurableStepOutcome::Failed("instance kernel already consumed".to_owned())
            }
        };
        let mut driver = DoInstanceDriver {
            kernel,
            files: self.files.as_ref(),
            coerce: self.coerce.as_ref(),
            agent_model: self.agent_model.as_deref(),
            agent_tools: self.agent_tools.as_ref(),
            ir: &self.ir,
            instance_id: &self.instance_id,
        };

        let outcome = drive_fixpoint(&mut driver, &mut self.in_flight, incoming);
        self.kernel = Some(driver.kernel);
        outcome
    }

    /// The instance's durable status (`"running"` / `"completed"` / `"failed"` / …),
    /// for the worker to expose or to decide whether to keep the object warm.
    pub fn status(&self) -> Result<Option<String>, StoreError> {
        let kernel = self.kernel.as_ref().expect("kernel present between steps");
        Ok(kernel
            .store()
            .status(&self.instance_id)?
            .map(|status| status.instance.status))
    }

    /// Whether coerce is configured (mirrors a live worker's binding check).
    pub fn coerce_provider(&self) -> Option<CoerceProvider> {
        self.coerce.as_ref().map(|config| config.provider)
    }
}

/// The `InstanceStepMachine` fixpoint, factored out so it can borrow `in_flight`
/// disjointly from the handle's other fields.
fn drive_fixpoint<D: InstanceDriver>(
    driver: &mut D,
    in_flight: &mut Option<ClaimableEffect>,
    incoming: Option<Result<HttpResponse, TransportError>>,
) -> DurableStepOutcome {
    // Resume an effect suspended on an HTTP round with the host's response.
    if let Some(effect) = in_flight.take() {
        match driver.run_effect(&effect, incoming) {
            Ok(EffectStep::Done(_)) => {}
            Ok(EffectStep::NeedsHttp(request)) => {
                *in_flight = Some(effect);
                return DurableStepOutcome::NeedsHttp(request);
            }
            Err(error) => return DurableStepOutcome::Failed(format!("{error:?}")),
        }
    }

    loop {
        match driver.advance_rules() {
            Ok(true) => return DurableStepOutcome::Terminal,
            Ok(false) => {}
            Err(error) => return DurableStepOutcome::Failed(format!("{error:?}")),
        }
        let ready = match driver.next_ready_effect() {
            Ok(Some(effect)) => effect,
            Ok(None) => return DurableStepOutcome::Parked,
            Err(error) => return DurableStepOutcome::Failed(format!("{error:?}")),
        };
        match driver.run_effect(&ready, None) {
            Ok(EffectStep::Done(_)) => continue,
            Ok(EffectStep::NeedsHttp(request)) => {
                *in_flight = Some(ready);
                return DurableStepOutcome::NeedsHttp(request);
            }
            Err(error) => return DurableStepOutcome::Failed(format!("{error:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::do_store::test_support::store;

    /// The worker-shell loop over an effect-free workflow: `create`, then `step`
    /// until a terminal — no HTTP round, one settle.
    #[test]
    fn durable_instance_drives_an_effect_free_workflow_to_terminal() {
        let source = "workflow MinimalNoop\n\noutput result StartupSeen\n\n\
             class StartupSeen {\n  source string\n  state \"observed\"\n}\n\n\
             rule observe_start\n  when started\n=> {\n\
             \x20 record StartupSeen {\n    source \"external.started\"\n    state \"observed\"\n  }\n\n\
             \x20 complete result {\n    source \"external.started\"\n    state \"observed\"\n  }\n}\n";
        let mut instance = DurableInstance::create(
            store().sql,
            source,
            "{}",
            "local/MinimalNoop",
            DurableEffectPorts::default(),
        )
        .expect("create");
        assert!(
            matches!(instance.step(None), DurableStepOutcome::Terminal),
            "the worker drives the instance to its terminal in one step"
        );
        assert_eq!(
            instance.status().expect("status").as_deref(),
            Some("completed")
        );
    }

    /// The worker-shell loop over a COERCE workflow: `create`, `step(None)` yields
    /// NeedsHttp (the provider request), the shell performs the `fetch`, and
    /// `step(response)` settles to the terminal — the real durable-object suspend/
    /// resume across separate JS calls, over the handle.
    #[test]
    fn durable_instance_suspends_on_fetch_and_resumes_to_terminal() {
        use whipplescript_kernel::coerce_native::CoerceProvider;
        use whipplescript_kernel::sansio::HttpResponse;

        let source = "workflow CoerceScore\n\noutput result Decision\n\n\
             class Decision {\n  score float\n}\n\n\
             coerce scoreIt() -> Decision {\n  prompt \"\"\"\n  Score it.\n  {{ ctx.output_format }}\n  \"\"\"\n}\n\n\
             rule go\n  when started\n=> {\n  coerce scoreIt() as review\n\
             \x20 after review succeeds as decision {\n    complete result { score decision.score }\n  }\n\
             \x20 after review fails {\n    complete result { score 0.0 }\n  }\n}\n";
        let base = store();
        for stmt in [
            "INSERT INTO capability_schemas (capability, description, schema_json) \
             VALUES ('schema.coerce', 'Coerce.', '{}')",
            "INSERT INTO effect_providers (provider_id, effect_kind, provider, capability, config_json) \
             VALUES ('provider_coerce_builtin', 'schema.coerce', 'builtin-coerce', 'schema.coerce', '{}')",
            "INSERT INTO capability_bindings (binding_id, program_id, capability, provider, config_json) \
             VALUES ('binding_coerce_builtin', NULL, 'schema.coerce', 'builtin-coerce', '{}')",
        ] {
            base.sql.execute(stmt, &[]).expect("seed coerce provider");
        }

        let ports = DurableEffectPorts {
            coerce: Some(CoerceProviderConfig {
                provider: CoerceProvider::Anthropic,
                provider_name: "anthropic".to_owned(),
                base_url: "https://api.anthropic.com".to_owned(),
                api_key: "test-key".to_owned(),
                model: "claude-test".to_owned(),
                max_tokens: 1024,
            }),
            ..DurableEffectPorts::default()
        };
        let mut instance =
            DurableInstance::create(base.sql, source, "{}", "local/CoerceScore", ports)
                .expect("create");

        // First step: the coerce effect suspends on `fetch`.
        let request = match instance.step(None) {
            DurableStepOutcome::NeedsHttp(request) => request,
            other => panic!("expected NeedsHttp, got {other:?}"),
        };
        assert!(
            request.url.contains("anthropic"),
            "request targets the provider"
        );

        // The worker performs the fetch; feed a canned structured output back.
        let response = HttpResponse {
            status: 200,
            body: serde_json::json!({
                "content": [{ "type": "tool_use", "name": "Decision", "input": { "score": 0.9 } }],
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            }),
        };
        assert!(
            matches!(
                instance.step(Some(Ok(response))),
                DurableStepOutcome::Terminal
            ),
            "the resume settles the coerce and reaches the terminal"
        );
        assert_eq!(
            instance.status().expect("status").as_deref(),
            Some("completed")
        );
    }

    /// The worker-shell loop over an AGENT workflow, driving the real
    /// `MessagesApiClient` (the config-only, transport-free model client a live
    /// worker builds from its secrets): `create`, `step(None)` yields NeedsHttp
    /// carrying the REAL Anthropic messages request, the shell performs the `fetch`,
    /// and `step(response)` parses a final reply and settles to the terminal. This
    /// is the agent counterpart to the coerce test — the multi-round turn's first
    /// round suspends over the handle and resumes to a terminal, over the real wire
    /// format, not a fake model.
    #[test]
    fn durable_instance_runs_an_agent_turn_over_fetch_with_the_real_model_client() {
        use whipplescript_kernel::coerce_native::CoerceProvider;
        use whipplescript_kernel::harness_model::MessagesApiClient;
        use whipplescript_kernel::sansio::HttpResponse;

        let source = "workflow AgentDemo\n\noutput result Done\n\n\
             class Done {\n  ok int\n}\n\n\
             agent helper {\n  provider owned\n  profile \"repo-reader\"\n  capacity 1\n}\n\n\
             rule go\n  when started\n=> {\n  tell helper as reply \"\"\"\n  Do the thing.\n  \"\"\"\n\n\
             \x20 after reply succeeds {\n    complete result { ok 1 }\n  }\n\n\
             \x20 after reply fails {\n    complete result { ok 0 }\n  }\n}\n";
        let base = store();
        for stmt in [
            "INSERT INTO capability_schemas (capability, description, schema_json) \
             VALUES ('agent.tell', 'Run an agent turn.', '{}')",
            "INSERT INTO effect_providers (provider_id, effect_kind, provider, capability, config_json) \
             VALUES ('provider_agent_tell_builtin', 'agent.tell', 'builtin-agent-harness', 'agent.tell', '{}')",
            "INSERT INTO capability_bindings (binding_id, program_id, capability, provider, config_json) \
             VALUES ('binding_agent_tell_builtin', NULL, 'agent.tell', 'builtin-agent-harness', '{}')",
            "INSERT INTO profiles (profile_id, name, description, enforcement_mode, allowed_capabilities, config_json) \
             VALUES ('profile_repo_reader', 'repo-reader', 'reads', 'enforce', '[\"agent.tell\"]', '{}')",
        ] {
            base.sql.execute(stmt, &[]).expect("seed agent provider");
        }

        let ports = DurableEffectPorts {
            agent_model: Some(Box::new(MessagesApiClient::new(
                CoerceProvider::Anthropic,
                "test-key",
                "claude-test",
                "https://api.anthropic.com",
                1024,
            ))),
            ..DurableEffectPorts::default()
        };
        let mut instance =
            DurableInstance::create(base.sql, source, "{}", "local/AgentDemo", ports)
                .expect("create");

        // First step: the agent turn's first model call suspends on `fetch`, and the
        // request is the real Anthropic messages call the model client built.
        let request = match instance.step(None) {
            DurableStepOutcome::NeedsHttp(request) => request,
            other => panic!("expected NeedsHttp, got {other:?}"),
        };
        assert!(
            request.url.contains("anthropic") && request.url.ends_with("/v1/messages"),
            "request targets the Anthropic messages endpoint: {}",
            request.url
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(k, _)| k == "x-api-key" || k == "anthropic-version"),
            "request carries the Anthropic auth/version headers"
        );

        // The worker performs the fetch; feed a canned final reply (no tool calls).
        let response = HttpResponse {
            status: 200,
            body: serde_json::json!({
                "content": [{ "type": "text", "text": "did the thing" }],
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            }),
        };
        assert!(
            matches!(
                instance.step(Some(Ok(response))),
                DurableStepOutcome::Terminal
            ),
            "the resume parses the final reply, settles the turn, and reaches the terminal"
        );
        assert_eq!(
            instance.status().expect("status").as_deref(),
            Some("completed")
        );
    }
}
