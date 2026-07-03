//! The durable-object binding of the instance step machine (DR-0033 chunk 5c).
//!
//! `DoInstanceDriver` is the DO counterpart to the native `NativeInstanceDriver`:
//! it implements the kernel's [`InstanceDriver`] seam over one held
//! `RuntimeKernel<DoSqliteStore<Sql>>`, so the same [`InstanceStepMachine`] drives
//! a workflow instance on the durable object. Because `DoSqliteStore` now
//! implements all three store traits (chunk 5a), the whole rule pass
//! (`step_instance_generic`) runs over the DO's one SQLite.
//!
//! What is wired here: the rule pass (`advance_rules`) and ready-effect discovery
//! (`next_ready_effect`), both pure store work over the DO SQLite. Effect
//! *execution* (`run_effect`) is where the DO's HTTP effects will suspend with
//! `EffectStep::NeedsHttp` and be fulfilled through the isolate's `fetch` — that
//! wiring (the DO-reachable handler cores + `FetchHost`) is chunk 5b, so for now
//! `run_effect` surfaces a clear "not yet executable on the DO" error rather than
//! a silent skip. Effect-free workflows (a rule that reaches a terminal) already
//! drive end to end, proving the DO runs the scheduler over its store.

use whipplescript_kernel::effect_config::EffectConfig;
use whipplescript_kernel::effect_handlers::run_event_effect_generic;
use whipplescript_kernel::instance_machine::{EffectStep, InstanceDriver};
use whipplescript_kernel::rule_pass::step_instance_generic;
use whipplescript_kernel::sansio::{HttpResponse, TransportError};
use whipplescript_kernel::RuntimeKernel;
use whipplescript_parser::IrProgram;
use whipplescript_store::{ClaimableEffect, RuntimeStore, StoreError};

use crate::do_store::{DoSql, DoSqliteStore};

/// Drives a workflow instance's rule pass + effect discovery on the durable object.
pub struct DoInstanceDriver<'a, Sql: DoSql> {
    /// One held kernel over the DO's SQLite (backs runtime + coordination +
    /// work-items surfaces).
    pub kernel: RuntimeKernel<DoSqliteStore<Sql>>,
    pub ir: &'a IrProgram,
    pub instance_id: &'a str,
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
        _incoming: Option<Result<HttpResponse, TransportError>>,
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
}
