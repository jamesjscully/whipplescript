use std::fs;

use whipplescript_kernel::{
    coerce::{BamlCoerceRequest, FakeBamlClient},
    harness::MockAgentHarness,
    idempotency_key,
    loft::{FakeLoftClient, LoftAction, LoftEffectRequest},
    trace::check_trace,
    AgentTurnExecution, BamlCoerceExecution, HumanAskExecution, LoftEffectExecution,
    ProgramVersionInput, RuntimeKernel,
};
use whipplescript_parser::compile_program;
use whipplescript_store::{
    EffectCompletion, NewEffect, NewEffectDependency, NewFact, NewWorkflowInvocation,
    ProgramVersionRecord, RevisionActivation, RuleCommit, RunStart, SqliteStore, StoreError,
    WorkflowTerminal, WorkflowTerminalKind,
};

#[test]
fn e2e_compiles_and_runs_minimal_workflow() {
    let source = include_str!("../../../examples/minimal-noop.whip");
    let (mut kernel, instance_id) = kernel_from_source("MinimalNoop", source);
    let event = kernel
        .ingest_external_event(&instance_id, "external.started", "{}", Some("start"))
        .expect("external event ingests");
    let facts = [NewFact {
        fact_id: "fact-startup-seen",
        name: "StartupSeen",
        key: "external.started",
        value_json: r#"{"source":"external.started","state":"observed"}"#,
        schema_id: Some("StartupSeen"),
        provenance_class: "rule",
        correlation_id: None,
    }];

    kernel
        .commit_rule(RuleCommit {
            instance_id: &instance_id,
            rule: "observe_start",
            trigger_event_id: Some(&event.event_id),
            facts: &facts,
            consumed_fact_ids: &[],
            effects: &[],
            dependencies: &[],
            terminal: None,
            idempotency_key: Some("commit-observe-start"),
        })
        .expect("minimal rule commits");

    assert_e2e_trace("minimal", &kernel);
    let store = kernel.into_store();
    let facts = store.list_facts(&instance_id).expect("facts list");
    assert!(facts
        .iter()
        .any(|fact| fact.name == "StartupSeen" && fact.value_json.contains("observed")));
}

#[test]
fn e2e_loft_claim_success_runs_agent_after_claim() {
    let source = include_str!("../../../examples/loft-worker-with-review.whip");
    let (mut kernel, instance_id) = kernel_from_source("LoftWorkerWithReview", source);
    let effects = [
        effect("claim", "loft.claim", r#"{"issue_id":"iss_abc"}"#),
        effect("tell", "agent.tell", r#"{"prompt":"implement"}"#),
    ];
    let dependencies = [dependency("dep-claim-tell", "claim", "succeeds", "tell")];
    kernel
        .commit_rule(RuleCommit {
            instance_id: &instance_id,
            rule: "start_ready_issue",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &effects,
            dependencies: &dependencies,
            terminal: None,
            idempotency_key: Some("commit-start-ready-issue"),
        })
        .expect("start rule commits");

    let request = loft_claim_request("iss_abc", "cmd-claim");
    kernel
        .run_loft_effect(
            LoftEffectExecution {
                instance_id: &instance_id,
                effect_id: "claim",
                run_id: "run-claim",
                provider: "fake-loft",
                worker_id: "worker-1",
                lease_id: "lease-claim",
                lease_expires_at: "2030-01-01T00:00:00Z",
                request: &request,
            },
            &FakeLoftClient::succeeds(
                r#"{"lease_id":"lea_abc","issue":{"id":"iss_abc","state_token":"ok"},"expires_at":"2030-01-01T00:00:00Z"}"#,
            ),
        )
        .expect("claim succeeds");

    kernel
        .run_agent_turn(
            AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-tell",
                provider: "mock-agent",
                worker_id: "worker-1",
                lease_id: "lease-tell",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "worker",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"implement"}"#,
                skill_names: &[],
            },
            &MockAgentHarness::completed("implemented"),
        )
        .expect("agent turn runs after claim");

    assert_e2e_trace("loft-claim-success", &kernel);
    let store = kernel.into_store();
    let events = store.list_events(&instance_id).expect("events list");
    let claim_terminal = event_sequence(&events, "effect.terminal", "claim");
    let tell_started = event_sequence(&events, "effect.run_started", "tell");
    assert!(claim_terminal < tell_started);
    let facts = store.list_facts(&instance_id).expect("facts list");
    assert!(facts.iter().any(|fact| fact.name == "loft.claim.succeeded"));
    assert!(facts.iter().any(|fact| fact.name == "agent.turn.completed"));
}

#[test]
fn e2e_loft_claim_failure_routes_to_human_review() {
    let source = include_str!("../../../examples/loft-worker-with-review.whip");
    let (mut kernel, instance_id) = kernel_from_source("LoftWorkerWithReview", source);
    let effects = [
        effect("claim", "loft.claim", r#"{"issue_id":"iss_busy"}"#),
        effect("review", "human.ask", r#"{"prompt":"claim failed"}"#),
    ];
    let dependencies = [dependency("dep-claim-review", "claim", "fails", "review")];
    kernel
        .commit_rule(RuleCommit {
            instance_id: &instance_id,
            rule: "start_ready_issue",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &effects,
            dependencies: &dependencies,
            terminal: None,
            idempotency_key: Some("commit-claim-failure"),
        })
        .expect("claim failure rule commits");

    let request = loft_claim_request("iss_busy", "cmd-claim-busy");
    kernel
        .run_loft_effect(
            LoftEffectExecution {
                instance_id: &instance_id,
                effect_id: "claim",
                run_id: "run-claim",
                provider: "fake-loft",
                worker_id: "worker-1",
                lease_id: "lease-claim",
                lease_expires_at: "2030-01-01T00:00:00Z",
                request: &request,
            },
            &FakeLoftClient::fails("issue already leased"),
        )
        .expect("claim failure records");

    kernel
        .run_human_ask(HumanAskExecution {
            instance_id: &instance_id,
            effect_id: "review",
            run_id: "run-review",
            provider: "builtin-human-review",
            worker_id: "worker-1",
            lease_id: "lease-review",
            lease_expires_at: "2030-01-01T00:00:00Z",
            inbox_item_id: "inbox-review",
            prompt: "Claim failed; inspect the issue.",
            choices_json: r#"["retry","block"]"#,
            freeform_allowed: true,
            severity: "warning",
            related_effects_json: r#"["claim"]"#,
            related_artifacts_json: "[]",
        })
        .expect("human review requested");

    assert_e2e_trace("loft-claim-failure", &kernel);
    let store = kernel.into_store();
    let facts = store.list_facts(&instance_id).expect("facts list");
    assert!(facts.iter().any(|fact| fact.name == "loft.claim.failed"));
    assert!(facts.iter().any(|fact| fact.name == "human.ask.created"));
    let inbox = store.list_inbox_items(None).expect("inbox list");
    assert_eq!(inbox.len(), 1);
}

#[test]
fn e2e_coerce_success_and_failure_branches_are_deterministic() {
    let source = include_str!("../../../examples/coerce-branch.whip");
    let (mut success_kernel, success_instance) = kernel_from_source("CoerceBranch", source);
    commit_single_effect(
        &mut success_kernel,
        &success_instance,
        effect(
            "classification",
            "baml.coerce",
            r#"{"function_name":"classifyMessage"}"#,
        ),
        "classify_request",
    );
    let request = coerce_request();
    success_kernel
        .run_baml_coerce(
            BamlCoerceExecution {
                instance_id: &success_instance,
                effect_id: "classification",
                run_id: "run-classification",
                provider: "fake-baml",
                worker_id: "worker-1",
                lease_id: "lease-classification",
                lease_expires_at: "2030-01-01T00:00:00Z",
                request: &request,
            },
            &FakeBamlClient::succeeds(
                r#"{"priority":"Urgent","summary":"triage now","confidence":0.99}"#,
            ),
        )
        .expect("coerce succeeds");
    assert_e2e_trace("coerce-success", &success_kernel);
    let success_store = success_kernel.into_store();
    assert!(success_store
        .list_facts(&success_instance)
        .expect("facts list")
        .iter()
        .any(|fact| fact.name == "baml.coerce.succeeded"));

    let (mut failure_kernel, failure_instance) = kernel_from_source("CoerceBranch", source);
    let effects = [
        effect(
            "classification",
            "baml.coerce",
            r#"{"function_name":"classifyMessage"}"#,
        ),
        effect("fallback", "human.ask", r#"{"prompt":"classify manually"}"#),
    ];
    let dependencies = [dependency(
        "dep-classification-fallback",
        "classification",
        "fails",
        "fallback",
    )];
    failure_kernel
        .commit_rule(RuleCommit {
            instance_id: &failure_instance,
            rule: "classify_request",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &effects,
            dependencies: &dependencies,
            terminal: None,
            idempotency_key: Some("commit-classify-failure"),
        })
        .expect("coerce failure rule commits");
    failure_kernel
        .run_baml_coerce(
            BamlCoerceExecution {
                instance_id: &failure_instance,
                effect_id: "classification",
                run_id: "run-classification",
                provider: "fake-baml",
                worker_id: "worker-1",
                lease_id: "lease-classification",
                lease_expires_at: "2030-01-01T00:00:00Z",
                request: &request,
            },
            &FakeBamlClient::fails("invalid classification"),
        )
        .expect("coerce failure records");
    failure_kernel
        .run_human_ask(HumanAskExecution {
            instance_id: &failure_instance,
            effect_id: "fallback",
            run_id: "run-fallback",
            provider: "builtin-human-review",
            worker_id: "worker-1",
            lease_id: "lease-fallback",
            lease_expires_at: "2030-01-01T00:00:00Z",
            inbox_item_id: "inbox-fallback",
            prompt: "Classify manually.",
            choices_json: r#"["low","normal","urgent"]"#,
            freeform_allowed: true,
            severity: "warning",
            related_effects_json: r#"["classification"]"#,
            related_artifacts_json: "[]",
        })
        .expect("fallback human review requested");
    assert_e2e_trace("coerce-failure", &failure_kernel);
    let failure_store = failure_kernel.into_store();
    let facts = failure_store
        .list_facts(&failure_instance)
        .expect("facts list");
    assert!(facts.iter().any(|fact| fact.name == "baml.coerce.failed"));
    assert!(facts.iter().any(|fact| fact.name == "human.ask.created"));
}

#[test]
fn e2e_concurrent_instances_do_not_cross_contaminate_facts() {
    let source = include_str!("../../../examples/minimal-noop.whip");
    let compiled = compile_program(source);
    assert_eq!(compiled.diagnostics, Vec::new());
    let ir = compiled.ir.expect("example compiles");
    let store = SqliteStore::open_in_memory().expect("store opens");
    let mut kernel = RuntimeKernel::new(store);
    let version = kernel
        .create_program_version(ProgramVersionInput {
            program_name: &ir.workflow,
            source_hash: "source",
            ir_hash: "ir",
            compiler_version: "e2e",
        })
        .expect("program version creates");
    let first = kernel
        .create_instance(&version, r#"{"ticket":"one"}"#)
        .expect("first instance creates");
    let second = kernel
        .create_instance(&version, r#"{"ticket":"two"}"#)
        .expect("second instance creates");

    for instance_id in [&first, &second] {
        let key = format!("startup-{instance_id}");
        let value = format!(r#"{{"source":"{instance_id}","state":"observed"}}"#);
        let facts = [NewFact {
            fact_id: &key,
            name: "StartupSeen",
            key: instance_id,
            value_json: &value,
            schema_id: Some("StartupSeen"),
            provenance_class: "rule",
            correlation_id: None,
        }];
        kernel
            .commit_rule(RuleCommit {
                instance_id,
                rule: "observe_start",
                trigger_event_id: None,
                facts: &facts,
                consumed_fact_ids: &[],
                effects: &[],
                dependencies: &[],
                terminal: None,
                idempotency_key: Some(&idempotency_key(&[instance_id, "observe_start"])),
            })
            .expect("rule commits");
    }

    assert_e2e_trace("multi-instance", &kernel);
    let store = kernel.into_store();
    let first_facts = store.list_facts(&first).expect("first facts list");
    let second_facts = store.list_facts(&second).expect("second facts list");
    assert!(first_facts
        .iter()
        .all(|fact| fact.value_json.contains(&first)));
    assert!(second_facts
        .iter()
        .all(|fact| fact.value_json.contains(&second)));
}

#[test]
fn e2e_lease_expiry_and_retry_recover_effects() {
    let source = include_str!("../../../examples/ralph.whip");
    let (mut kernel, instance_id) = kernel_from_source("Ralph", source);
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("tell", "agent.tell", r#"{"prompt":"go"}"#),
        "begin",
    );

    kernel
        .start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "tell",
            run_id: "run-tell-1",
            provider: "mock-agent",
            worker_id: "worker-1",
            lease_id: "lease-tell-1",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("first run starts");
    let expired = kernel
        .expire_leases(&instance_id, "2030-01-02T00:00:00Z")
        .expect("lease expires");
    assert_eq!(expired.len(), 1);

    kernel
        .start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "tell",
            run_id: "run-tell-2",
            provider: "mock-agent",
            worker_id: "worker-1",
            lease_id: "lease-tell-2",
            lease_expires_at: "2030-01-03T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("second run starts after expiry");
    kernel
        .fail_run(EffectCompletion {
            instance_id: &instance_id,
            effect_id: "tell",
            run_id: "run-tell-2",
            provider: "mock-agent",
            worker_id: "worker-1",
            status: "ignored",
            exit_code: Some(1),
            summary: Some("transient failure"),
            metadata_json: "{}",
            idempotency_key: Some("fail-run-tell-2"),
        })
        .expect("second run fails");
    kernel
        .retry_effect(whipplescript_store::RetryEffect {
            instance_id: &instance_id,
            effect_id: "tell",
            retry_after: None,
            idempotency_key: Some("retry-tell"),
        })
        .expect("effect retries");

    assert_e2e_trace("lease-retry", &kernel);
    let store = kernel.into_store();
    let effects = store.list_effects(&instance_id).expect("effects list");
    assert_eq!(effects[0].status, "queued");
}

#[test]
fn e2e_capability_denial_blocks_with_useful_status() {
    let source = include_str!("../../../examples/plugin-memory.whip");
    let (mut kernel, instance_id) = kernel_from_source("PluginMemory", source);
    let denied = NewEffect {
        effect_id: "write",
        kind: "agent.tell",
        target: None,
        input_json: r#"{"prompt":"write"}"#,
        status: "queued",
        idempotency_key: "write",
        required_capabilities_json: r#"["repo.write"]"#,
        profile: Some("repo-reader"),
        correlation_id: None,
        source_span_json: None,
    };
    commit_single_effect(&mut kernel, &instance_id, denied, "deny_write");

    let blocked = kernel.start_run(RunStart {
        instance_id: &instance_id,
        effect_id: "write",
        run_id: "run-write",
        provider: "mock-agent",
        worker_id: "worker-1",
        lease_id: "lease-write",
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: "{}",
    });

    assert!(matches!(blocked, Err(StoreError::PolicyBlocked { .. })));
    assert_e2e_trace("capability-denial", &kernel);
    let store = kernel.into_store();
    let effects = store.list_effects(&instance_id).expect("effects list");
    assert_eq!(effects[0].status, "blocked_by_profile");
    assert!(effects[0]
        .policy_block_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("repo.write")));
}

#[test]
fn e2e_plugin_registered_effect_runs_through_outbox() {
    let source = include_str!("../../../examples/plugin-memory.whip");
    let compiled = compile_program(source);
    assert_eq!(compiled.diagnostics, Vec::new());
    let ir = compiled.ir.expect("source compiles");
    let store = SqliteStore::open_in_memory().expect("store opens");
    store
        .register_plugin_manifest(include_str!("../../../examples/plugins/memory.json"))
        .expect("memory plugin registers");
    let mut kernel = RuntimeKernel::new(store);
    let version = kernel
        .create_program_version(ProgramVersionInput {
            program_name: &ir.workflow,
            source_hash: "source",
            ir_hash: "ir",
            compiler_version: "e2e",
        })
        .expect("program version creates");
    let instance_id = kernel
        .create_instance(&version, "{}")
        .expect("instance creates");
    let memory_query = NewEffect {
        effect_id: "context",
        kind: "memory.query",
        target: None,
        input_json: r#"{"query":"issue context"}"#,
        status: "queued",
        idempotency_key: "context",
        required_capabilities_json: r#"["memory.query"]"#,
        profile: Some("memory-user"),
        correlation_id: None,
        source_span_json: None,
    };
    commit_single_effect(
        &mut kernel,
        &instance_id,
        memory_query,
        "recall_before_work",
    );

    kernel
        .start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "context",
            run_id: "run-context",
            provider: "memory-plugin",
            worker_id: "worker-1",
            lease_id: "lease-context",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("plugin effect starts");
    kernel
        .complete_run(EffectCompletion {
            instance_id: &instance_id,
            effect_id: "context",
            run_id: "run-context",
            provider: "memory-plugin",
            worker_id: "worker-1",
            status: "ignored",
            exit_code: Some(0),
            summary: Some("memory query completed"),
            metadata_json: r#"{"records":[]}"#,
            idempotency_key: Some("complete-context"),
        })
        .expect("plugin effect completes");

    assert_e2e_trace("plugin-effect", &kernel);
    let store = kernel.into_store();
    let effects = store.list_effects(&instance_id).expect("effects list");
    assert_eq!(effects[0].kind, "memory.query");
    assert_eq!(effects[0].status, "completed");
}

#[test]
fn e2e_ralph_loop_runs_one_bounded_followup_turn() {
    let source = include_str!("../../../examples/ralph.whip");
    let (mut kernel, instance_id) = kernel_from_source("Ralph", source);
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("begin", "agent.tell", r#"{"prompt":"first"}"#),
        "begin",
    );
    kernel
        .run_agent_turn(
            AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "begin",
                run_id: "run-begin",
                provider: "mock-agent",
                worker_id: "worker-1",
                lease_id: "lease-begin",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "ralph",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"first"}"#,
                skill_names: &[],
            },
            &MockAgentHarness::completed("first turn"),
        )
        .expect("first ralph turn runs");
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("again", "agent.tell", r#"{"prompt":"second"}"#),
        "again",
    );
    kernel
        .run_agent_turn(
            AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "again",
                run_id: "run-again",
                provider: "mock-agent",
                worker_id: "worker-1",
                lease_id: "lease-again",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "ralph",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"second"}"#,
                skill_names: &[],
            },
            &MockAgentHarness::completed("second turn"),
        )
        .expect("bounded followup turn runs");

    assert_e2e_trace("ralph-bounded", &kernel);
    let store = kernel.into_store();
    let facts = store.list_facts(&instance_id).expect("facts list");
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.name == "agent.turn.completed")
            .count(),
        2
    );
}

#[test]
fn e2e_pause_resume_and_cancel_gate_provider_starts() {
    let source = include_str!("../../../examples/ralph.whip");
    let (mut kernel, instance_id) = kernel_from_source("Ralph", source);
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("tell", "agent.tell", r#"{"prompt":"go"}"#),
        "begin",
    );

    kernel
        .pause_instance(&instance_id, Some("operator"), Some("pause"))
        .expect("instance pauses");
    assert!(kernel
        .claimable_effects(&instance_id)
        .expect("claimable effects")
        .is_empty());
    let paused_start = kernel.start_run(RunStart {
        instance_id: &instance_id,
        effect_id: "tell",
        run_id: "run-paused",
        provider: "mock-agent",
        worker_id: "worker-1",
        lease_id: "lease-paused",
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: "{}",
    });
    assert!(
        matches!(paused_start, Err(StoreError::Conflict(message)) if message.contains("paused"))
    );

    kernel
        .resume_instance(&instance_id, Some("resume"))
        .expect("instance resumes");
    kernel
        .run_agent_turn(
            AgentTurnExecution {
                instance_id: &instance_id,
                effect_id: "tell",
                run_id: "run-resumed",
                provider: "mock-agent",
                worker_id: "worker-1",
                lease_id: "lease-resumed",
                lease_expires_at: "2030-01-01T00:00:00Z",
                agent: "ralph",
                profile: Some("repo-writer"),
                input_json: r#"{"prompt":"go"}"#,
                skill_names: &[],
            },
            &MockAgentHarness::completed("resumed"),
        )
        .expect("resumed instance runs");

    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("cancelled-tell", "agent.tell", r#"{"prompt":"stop"}"#),
        "again",
    );
    kernel
        .cancel_instance(&instance_id, Some("operator"), Some("cancel"))
        .expect("instance cancels");
    let cancelled_start = kernel.start_run(RunStart {
        instance_id: &instance_id,
        effect_id: "cancelled-tell",
        run_id: "run-cancelled",
        provider: "mock-agent",
        worker_id: "worker-1",
        lease_id: "lease-cancelled",
        lease_expires_at: "2030-01-01T00:00:00Z",
        metadata_json: "{}",
    });
    assert!(
        matches!(cancelled_start, Err(StoreError::Conflict(message)) if message.contains("cancelled"))
    );

    assert_e2e_trace("pause-resume-cancel", &kernel);
}

#[test]
fn e2e_restart_rebuilds_projection_from_event_log() {
    let source = include_str!("../../../examples/plugin-memory.whip");
    let (mut kernel, instance_id) = kernel_from_source("PluginMemory", source);
    let effects = [
        effect("context", "memory.query", r#"{"query":"issue"}"#),
        effect("tell", "agent.tell", r#"{"prompt":"use memory"}"#),
    ];
    let dependencies = [dependency(
        "dep-context-tell",
        "context",
        "succeeds",
        "tell",
    )];
    kernel
        .commit_rule(RuleCommit {
            instance_id: &instance_id,
            rule: "recall_before_work",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &effects,
            dependencies: &dependencies,
            terminal: None,
            idempotency_key: Some("commit-recall"),
        })
        .expect("rule commits before restart");

    assert_e2e_trace("restart-before", &kernel);
    let mut store = kernel.into_store();
    store
        .rebuild_projections(&instance_id)
        .expect("projections rebuild from event log");
    let restarted = RuntimeKernel::new(store);
    let store = restarted.into_store();
    assert_eq!(
        store
            .list_effects(&instance_id)
            .expect("effects list")
            .len(),
        2
    );
    assert_eq!(
        store.list_events(&instance_id).expect("events list").len(),
        1
    );
}

#[test]
fn e2e_repeated_dependency_claimability_stress() {
    for index in 0..25 {
        let source = include_str!("../../../examples/loft-worker-with-review.whip");
        let (mut kernel, instance_id) = kernel_from_source("LoftWorkerWithReview", source);
        let effects = [
            effect("claim", "loft.claim", r#"{"issue_id":"iss_stress"}"#),
            effect("tell", "agent.tell", r#"{"prompt":"implement"}"#),
        ];
        let dependencies = [dependency("dep-claim-tell", "claim", "succeeds", "tell")];
        kernel
            .commit_rule(RuleCommit {
                instance_id: &instance_id,
                rule: "start_ready_issue",
                trigger_event_id: None,
                facts: &[],
                consumed_fact_ids: &[],
                effects: &effects,
                dependencies: &dependencies,
                terminal: None,
                idempotency_key: Some(&format!("commit-stress-{index}")),
            })
            .expect("stress rule commits");

        let initial = kernel
            .claimable_effects(&instance_id)
            .expect("initial claimable effects");
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].effect_id, "claim");
        kernel
            .start_run(RunStart {
                instance_id: &instance_id,
                effect_id: "claim",
                run_id: "run-claim",
                provider: "fake-loft",
                worker_id: "worker-1",
                lease_id: "lease-claim",
                lease_expires_at: "2030-01-01T00:00:00Z",
                metadata_json: "{}",
            })
            .expect("claim starts");
        kernel
            .complete_run(EffectCompletion {
                instance_id: &instance_id,
                effect_id: "claim",
                run_id: "run-claim",
                provider: "fake-loft",
                worker_id: "worker-1",
                status: "ignored",
                exit_code: Some(0),
                summary: Some("claimed"),
                metadata_json: "{}",
                idempotency_key: Some("complete-claim"),
            })
            .expect("claim completes");
        let released = kernel
            .claimable_effects(&instance_id)
            .expect("released claimable effects");
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].effect_id, "tell");
        assert_e2e_trace(&format!("stress-{index}"), &kernel);
    }
}

#[test]
fn e2e_revision_keep_preserves_running_old_effect_and_changes_future_dispatch() {
    let (mut kernel, instance_id, version1, version2) = revision_kernel("RevisionKeepE2E");
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("old-turn", "agent.tell", r#"{"prompt":"old dispatch"}"#),
        "old_dispatch",
    );
    kernel
        .start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "old-turn",
            run_id: "run-old-turn",
            provider: "mock-agent",
            worker_id: "worker-1",
            lease_id: "lease-old-turn",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("old turn starts");

    kernel
        .activate_revision(RevisionActivation {
            instance_id: &instance_id,
            from_version_id: &version1.version_id,
            to_version_id: &version2.version_id,
            activation_policy_json: r#"{"test":"keep"}"#,
            cancellation_policy: "keep",
            idempotency_key: Some("e2e-revision-keep"),
        })
        .expect("revision activates");
    kernel
        .complete_run(EffectCompletion {
            instance_id: &instance_id,
            effect_id: "old-turn",
            run_id: "run-old-turn",
            provider: "mock-agent",
            worker_id: "worker-1",
            status: "ignored",
            exit_code: Some(0),
            summary: Some("old dispatch completed"),
            metadata_json: "{}",
            idempotency_key: Some("complete-old-turn"),
        })
        .expect("old turn completes after revision");
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("new-turn", "agent.tell", r#"{"prompt":"new dispatch"}"#),
        "new_dispatch",
    );

    assert_e2e_trace("revision-keep", &kernel);
    let store = kernel.into_store();
    let effects = store.list_effects(&instance_id).expect("effects list");
    let old = effects
        .iter()
        .find(|effect| effect.effect_id == "old-turn")
        .expect("old effect exists");
    let new = effects
        .iter()
        .find(|effect| effect.effect_id == "new-turn")
        .expect("new effect exists");
    assert_eq!(
        old.program_version_id.as_deref(),
        Some(version1.version_id.as_str())
    );
    assert_eq!(old.revision_epoch, 0);
    assert_eq!(
        new.program_version_id.as_deref(),
        Some(version2.version_id.as_str())
    );
    assert_eq!(new.revision_epoch, 1);
}

#[test]
fn e2e_revision_queued_cancel_terminal_cancels_old_effects() {
    let (mut kernel, instance_id, version1, version2) = revision_kernel("RevisionQueuedE2E");
    let effects = [
        effect("queued-turn", "agent.tell", r#"{"prompt":"queued"}"#),
        effect("after-cancel", "agent.tell", r#"{"prompt":"after"}"#),
    ];
    let dependencies = [dependency(
        "dep-queued-after",
        "queued-turn",
        "completes",
        "after-cancel",
    )];
    kernel
        .commit_rule(RuleCommit {
            instance_id: &instance_id,
            rule: "queued_dispatch",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &effects,
            dependencies: &dependencies,
            terminal: None,
            idempotency_key: Some("commit-queued-revision"),
        })
        .expect("queued effects commit");

    kernel
        .activate_revision(RevisionActivation {
            instance_id: &instance_id,
            from_version_id: &version1.version_id,
            to_version_id: &version2.version_id,
            activation_policy_json: r#"{"test":"queued"}"#,
            cancellation_policy: "queued",
            idempotency_key: Some("e2e-revision-queued"),
        })
        .expect("queued revision activates");

    assert_e2e_trace("revision-queued-cancel", &kernel);
    let store = kernel.into_store();
    let effects = store.list_effects(&instance_id).expect("effects list");
    let queued = effects
        .iter()
        .find(|effect| effect.effect_id == "queued-turn")
        .expect("queued effect exists");
    let after = effects
        .iter()
        .find(|effect| effect.effect_id == "after-cancel")
        .expect("downstream effect exists");
    assert_eq!(queued.status, "cancelled");
    assert_eq!(after.status, "cancelled");
}

#[test]
fn e2e_revision_running_cancel_request_allows_late_terminal() {
    let (mut kernel, instance_id, version1, version2) = revision_kernel("RevisionRunningE2E");
    commit_single_effect(
        &mut kernel,
        &instance_id,
        effect("running-turn", "agent.tell", r#"{"prompt":"running"}"#),
        "running_dispatch",
    );
    kernel
        .start_run(RunStart {
            instance_id: &instance_id,
            effect_id: "running-turn",
            run_id: "run-running-turn",
            provider: "mock-agent",
            worker_id: "worker-1",
            lease_id: "lease-running-turn",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("running turn starts");

    kernel
        .activate_revision(RevisionActivation {
            instance_id: &instance_id,
            from_version_id: &version1.version_id,
            to_version_id: &version2.version_id,
            activation_policy_json: r#"{"test":"running"}"#,
            cancellation_policy: "running",
            idempotency_key: Some("e2e-revision-running"),
        })
        .expect("running revision activates");
    kernel
        .fail_run(EffectCompletion {
            instance_id: &instance_id,
            effect_id: "running-turn",
            run_id: "run-running-turn",
            provider: "mock-agent",
            worker_id: "worker-1",
            status: "ignored",
            exit_code: Some(1),
            summary: Some("provider observed cancellation request"),
            metadata_json: "{}",
            idempotency_key: Some("fail-running-turn"),
        })
        .expect("running turn fails after cancellation request");

    assert_e2e_trace("revision-running-cancel-request", &kernel);
    let store = kernel.into_store();
    let effects = store.list_effects(&instance_id).expect("effects list");
    let running = effects
        .iter()
        .find(|effect| effect.effect_id == "running-turn")
        .expect("running effect exists");
    assert_eq!(running.status, "failed");
    assert!(!running.cancel_requested);
    let requests = store
        .list_effect_cancellation_requests(&instance_id)
        .expect("cancellation requests list");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, "terminal");
}

#[test]
fn e2e_parent_revision_preserves_running_child_invocation() {
    let (mut kernel, parent_id, parent_v1, parent_v2) = revision_kernel("ParentRevisionE2E");
    let child_v1 = revision_program_version(&mut kernel, "ChildRevisionE2E", "child_v1");
    let child_id = kernel
        .create_instance(&child_v1, r#"{"task":"child"}"#)
        .expect("child instance creates");
    commit_single_effect(
        &mut kernel,
        &parent_id,
        effect(
            "invoke-child",
            "workflow.invoke",
            r#"{"workflow":"ChildRevisionE2E"}"#,
        ),
        "invoke_child",
    );
    kernel
        .start_run(RunStart {
            instance_id: &parent_id,
            effect_id: "invoke-child",
            run_id: "run-invoke-child",
            provider: "workflow",
            worker_id: "worker-1",
            lease_id: "lease-invoke-child",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("parent invocation run starts");
    kernel
        .record_workflow_invocation(NewWorkflowInvocation {
            invocation_id: "inv-parent-running-child",
            parent_instance_id: &parent_id,
            parent_effect_id: "invoke-child",
            child_instance_id: &child_id,
            target_workflow: "ChildRevisionE2E",
            input_json: r#"{"task":"child"}"#,
            source_span_json: None,
            idempotency_key: "inv-parent-running-child",
        })
        .expect("invocation records");
    commit_single_effect(
        &mut kernel,
        &child_id,
        effect("child-turn", "agent.tell", r#"{"prompt":"child"}"#),
        "child_dispatch",
    );
    kernel
        .start_run(RunStart {
            instance_id: &child_id,
            effect_id: "child-turn",
            run_id: "run-child-turn",
            provider: "mock-agent",
            worker_id: "worker-1",
            lease_id: "lease-child-turn",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("child run starts");

    kernel
        .activate_revision(RevisionActivation {
            instance_id: &parent_id,
            from_version_id: &parent_v1.version_id,
            to_version_id: &parent_v2.version_id,
            activation_policy_json: r#"{"test":"parent"}"#,
            cancellation_policy: "keep",
            idempotency_key: Some("e2e-revision-parent"),
        })
        .expect("parent revision activates");

    assert_e2e_trace("revision-parent-child-running", &kernel);
    let invocation = kernel
        .get_workflow_invocation(&parent_id, "invoke-child")
        .expect("invocation loads")
        .expect("invocation exists");
    assert_eq!(
        invocation.parent_program_version_id.as_deref(),
        Some(parent_v1.version_id.as_str())
    );
    assert_eq!(invocation.parent_revision_epoch, 0);
    assert_eq!(
        invocation.parent_active_program_version_id.as_deref(),
        Some(parent_v2.version_id.as_str())
    );
    assert_eq!(invocation.parent_active_revision_epoch, Some(1));
    assert_eq!(
        invocation.child_program_version_id.as_deref(),
        Some(child_v1.version_id.as_str())
    );
    assert_eq!(invocation.child_active_revision_epoch, Some(0));
    assert_eq!(invocation.status, "running");
}

#[test]
fn e2e_child_revision_parent_observes_terminal_output() {
    let (mut kernel, parent_id, parent_v1, _parent_v2) = revision_kernel("ParentObserveE2E");
    let child_v1 = revision_program_version(&mut kernel, "ChildObserveE2E", "child_v1");
    let child_v2 = revision_program_version(&mut kernel, "ChildObserveE2E", "child_v2");
    let child_id = kernel
        .create_instance(&child_v1, r#"{"task":"child"}"#)
        .expect("child instance creates");
    commit_single_effect(
        &mut kernel,
        &parent_id,
        effect(
            "invoke-child",
            "workflow.invoke",
            r#"{"workflow":"ChildObserveE2E"}"#,
        ),
        "invoke_child",
    );
    kernel
        .start_run(RunStart {
            instance_id: &parent_id,
            effect_id: "invoke-child",
            run_id: "run-observe-child",
            provider: "workflow",
            worker_id: "worker-1",
            lease_id: "lease-observe-child",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("parent invocation run starts");
    kernel
        .record_workflow_invocation(NewWorkflowInvocation {
            invocation_id: "inv-parent-observe-child",
            parent_instance_id: &parent_id,
            parent_effect_id: "invoke-child",
            child_instance_id: &child_id,
            target_workflow: "ChildObserveE2E",
            input_json: r#"{"task":"child"}"#,
            source_span_json: None,
            idempotency_key: "inv-parent-observe-child",
        })
        .expect("invocation records");

    kernel
        .activate_revision(RevisionActivation {
            instance_id: &child_id,
            from_version_id: &child_v1.version_id,
            to_version_id: &child_v2.version_id,
            activation_policy_json: r#"{"test":"child"}"#,
            cancellation_policy: "keep",
            idempotency_key: Some("e2e-revision-child"),
        })
        .expect("child revision activates");
    kernel
        .commit_rule(RuleCommit {
            instance_id: &child_id,
            rule: "complete_child",
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &[],
            dependencies: &[],
            terminal: Some(WorkflowTerminal {
                kind: WorkflowTerminalKind::Completed,
                name: "result",
                payload_json: r#"{"summary":"done"}"#,
                idempotency_key: Some("child-terminal"),
            }),
            idempotency_key: Some("commit-child-terminal"),
        })
        .expect("child workflow completes");
    kernel
        .complete_run(EffectCompletion {
            instance_id: &parent_id,
            effect_id: "invoke-child",
            run_id: "run-observe-child",
            provider: "workflow",
            worker_id: "worker-1",
            status: "ignored",
            exit_code: Some(0),
            summary: Some("child workflow completed"),
            metadata_json: r#"{"terminal":{"result":{"summary":"done"}}}"#,
            idempotency_key: Some("complete-parent-invocation"),
        })
        .expect("parent observes child terminal output");

    assert_e2e_trace("revision-child-observed", &kernel);
    let store = kernel.into_store();
    let invocation = store
        .get_workflow_invocation(&parent_id, "invoke-child")
        .expect("invocation loads")
        .expect("invocation exists");
    assert_eq!(
        invocation.parent_program_version_id.as_deref(),
        Some(parent_v1.version_id.as_str())
    );
    assert_eq!(
        invocation.child_program_version_id.as_deref(),
        Some(child_v1.version_id.as_str())
    );
    assert_eq!(
        invocation.child_active_program_version_id.as_deref(),
        Some(child_v2.version_id.as_str())
    );
    assert_eq!(invocation.child_active_revision_epoch, Some(1));
    assert_eq!(invocation.status, "completed");
    let child = store
        .get_instance(&child_id)
        .expect("child loads")
        .expect("child exists");
    assert_eq!(child.status, "completed");
}

fn kernel_from_source(name: &str, source: &str) -> (RuntimeKernel, String) {
    let compiled = compile_program(source);
    assert_eq!(compiled.diagnostics, Vec::new());
    let ir = compiled.ir.expect("source compiles");
    assert_eq!(ir.workflow, name);
    let store = SqliteStore::open_in_memory().expect("store opens");
    let mut kernel = RuntimeKernel::new(store);
    let version = kernel
        .create_program_version(ProgramVersionInput {
            program_name: &ir.workflow,
            source_hash: "source",
            ir_hash: "ir",
            compiler_version: "e2e",
        })
        .expect("program version creates");
    let instance_id = kernel
        .create_instance(&version, "{}")
        .expect("instance creates");
    (kernel, instance_id)
}

fn revision_kernel(
    name: &str,
) -> (
    RuntimeKernel,
    String,
    ProgramVersionRecord,
    ProgramVersionRecord,
) {
    let store = SqliteStore::open_in_memory().expect("store opens");
    let mut kernel = RuntimeKernel::new(store);
    let version1 = revision_program_version(&mut kernel, name, "v1");
    let version2 = revision_program_version(&mut kernel, name, "v2");
    let instance_id = kernel
        .create_instance(&version1, "{}")
        .expect("instance creates");
    (kernel, instance_id, version1, version2)
}

fn revision_program_version(
    kernel: &mut RuntimeKernel,
    workflow_name: &str,
    label: &str,
) -> ProgramVersionRecord {
    let source = format!(
        r#"
workflow {workflow_name}

rule {label}_noop
=> {{
}}
"#
    );
    let compiled = compile_program(&source);
    assert_eq!(compiled.diagnostics, Vec::new());
    let ir = compiled.ir.expect("revision source compiles");
    kernel
        .create_program_version_for_program(
            ProgramVersionInput {
                program_name: &ir.workflow,
                source_hash: &format!("{label}-source"),
                ir_hash: &format!("{label}-ir"),
                compiler_version: "e2e",
            },
            &ir,
        )
        .expect("revision program version creates")
}

fn commit_single_effect(
    kernel: &mut RuntimeKernel,
    instance_id: &str,
    effect: NewEffect<'_>,
    rule: &str,
) {
    let commit_key = idempotency_key(&[instance_id, rule, effect.effect_id]);
    kernel
        .commit_rule(RuleCommit {
            instance_id,
            rule,
            trigger_event_id: None,
            facts: &[],
            consumed_fact_ids: &[],
            effects: &[effect],
            dependencies: &[],
            terminal: None,
            idempotency_key: Some(&commit_key),
        })
        .expect("single effect commits");
}

fn effect<'a>(effect_id: &'a str, kind: &'a str, input_json: &'a str) -> NewEffect<'a> {
    NewEffect {
        effect_id,
        kind,
        target: None,
        input_json,
        status: "queued",
        idempotency_key: effect_id,
        required_capabilities_json: "[]",
        profile: None,
        correlation_id: None,
        source_span_json: None,
    }
}

fn dependency<'a>(
    dependency_id: &'a str,
    upstream: &'a str,
    predicate: &'a str,
    downstream: &'a str,
) -> NewEffectDependency<'a> {
    NewEffectDependency {
        dependency_id,
        upstream_effect_id: upstream,
        predicate,
        downstream_effect_id: downstream,
    }
}

fn loft_claim_request(issue_id: &str, command_id: &str) -> LoftEffectRequest {
    LoftEffectRequest {
        action: LoftAction::Claim,
        issue_id: issue_id.to_owned(),
        lease_id: None,
        claim_ready: false,
        issue_version: None,
        actor: Some("agent-a".to_owned()),
        lease_duration_seconds: Some(1800),
        command_id: command_id.to_owned(),
        note: None,
        target_status: None,
        evidence_json: None,
        evidence_kind: None,
        evidence_artifact: None,
        evidence_data_path: None,
        resource_intent_json: None,
        release_after_failure: false,
        expect_heads: Vec::new(),
        metadata_json: "{}".to_owned(),
    }
}

fn coerce_request() -> BamlCoerceRequest {
    BamlCoerceRequest {
        function_name: "classifyMessage".to_owned(),
        arguments_json: r#"{"title":"pager","body":"production is down"}"#.to_owned(),
        output_type: "MessageClassification".to_owned(),
        generated_baml_source_hash: "baml-source".to_owned(),
        input_schema_hash: "input-schema".to_owned(),
        output_schema_hash: "output-schema".to_owned(),
    }
}

fn event_sequence(
    events: &[whipplescript_store::EventView],
    event_type: &str,
    effect_id: &str,
) -> i64 {
    events
        .iter()
        .find(|event| event.event_type == event_type && event.payload_json.contains(effect_id))
        .map(|event| event.sequence)
        .expect("event exists")
}

fn assert_e2e_trace(name: &str, kernel: &RuntimeKernel) {
    let path = std::env::temp_dir().join(format!(
        "whipplescript-e2e-{name}-{}-trace.txt",
        std::process::id()
    ));
    fs::write(&path, format!("{:#?}\n", kernel.trace())).expect("trace artifact writes");
    check_trace(kernel.trace()).unwrap_or_else(|violation| {
        panic!(
            "trace conformance failed for {name}; artifact={}: {:?}",
            path.display(),
            violation
        )
    });
}
