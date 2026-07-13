//! The improve engine: gauge scoring, pinned scenarios, and `whip improve`
//! campaigns (`spec/improve-design-note.md`; parent surface
//! `spec/experimentation-subsystem-research-note.md`).
//!
//! v1 shape, on the settled ground only:
//! - The campaign partition is expressed by which gauges you name (no
//!   modes): named gauges ascend toward better, unnamed gauges are guarded
//!   within indifference bands, `--sacrifice` releases, declared bars are
//!   always hard. Bare `whip improve` is repair mode.
//! - Acceptance is the dominance invariant of
//!   `models/maude/improve-acceptance.maude`: never surface a dominated
//!   candidate. Genuine tradeoffs are surfaced as decisions, never
//!   auto-resolved (the local utility model is future work — v1 always
//!   asks).
//! - Holdout sealing per `models/maude/improve-holdout.maude`: sealed
//!   scenarios never reach the proposer (aggregates only), promotion gates
//!   wear seals out (k=3, cumulative across campaigns), and below the floor
//!   the campaign runs anyway tagged `unheld-out` — progressive rigor,
//!   never entry rigor.
//! - Propose, don't apply: the terminal state is an evidence card per
//!   candidate; `whip adopt` is the explicit human act, and it refuses when
//!   mainline moved under the campaign (the certified-merge rebase is the
//!   principled upgrade).
//! - Candidate evaluation regenerates pinned scenarios in DISPOSABLE temp
//!   stores (never the workspace store); evidence rows carry
//!   `execution_mode = regen`. Egress-door diversion for regenerated runs
//!   inherits the versioned-workspace containment posture when it lands and
//!   is not enforced here yet.
//!
//! Honest v1 gaps (all tagged, none silent): `coerce` judges are declared
//! but not yet scoreable (parameter binding needs program context); `prompt`
//! judges need a configured native coerce provider and are skipped
//! (`judge-unscored`) without one; spend accounting records what is
//! derivable and the cap applies to recorded cost, with provider price
//! tables a follow-on.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use whipplescript_parser::{IrGauge, IrProgram, BUILTIN_GAUGES};
use whipplescript_store::improve::{
    fold_campaign_event, CampaignSummary, ImproveStore, NewEvidence, ScenarioRow,
};
use whipplescript_store::SqliteStore;

use crate::{emit_json, CliOptions};

/// Promotion-gate wear-out threshold (design note §8, k=3).
const WEAR_OUT_AT: i64 = 3;
/// Sealed fraction and floor (design note §8: 20% / floor 2). Below
/// MIN_SCENARIOS_FOR_SEALING the campaign runs `unheld-out`: sealing 2 of 3
/// scenarios would leave the proposer almost blind, which fabricates
/// neither rigor nor progress.
const SEALED_FRACTION: f64 = 0.2;
const SEALED_FLOOR: usize = 2;
const MIN_SCENARIOS_FOR_SEALING: usize = 4;
/// Internal stopping backstop for the propose→evaluate loop. Deliberately
/// NOT a surface: the operator's levers are the spend cap and the campaign
/// verbs, never a sample count.
const MAX_PROPOSAL_ROUNDS: usize = 4;
/// Default relative indifference band for the built-in resource gauges
/// (design note §2 amendment: their noise floor degenerates toward zero).
const RESOURCE_BAND_PERCENT: f64 = 5.0;
/// Minimum absolute band for quality gauges when the sample is too small
/// for a meaningful noise floor.
const QUALITY_BAND_FLOOR: f64 = 0.02;

pub(crate) fn improve_store_path() -> PathBuf {
    std::env::var("WHIPPLESCRIPT_IMPROVE_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".whipplescript/improve.sqlite"))
}

fn open_improve_store() -> Result<ImproveStore, String> {
    ImproveStore::open(improve_store_path())
        .map_err(|error| format!("failed to open improve store: {error:?}"))
}

fn program_hash(source: &str) -> String {
    crate::sha256_hex(source.as_bytes())
}

/// The standalone coerce request shell for judge/proposer turns: the
/// NativeCoerceClient carries the real prompt/schema; these identity fields
/// exist for effect-shaped callers and are unused on this path.
fn compile_failure_summary(failure: &crate::CompileFailure) -> String {
    match failure {
        crate::CompileFailure::Io(error) => error.to_string(),
        crate::CompileFailure::Diagnostics { diagnostics, .. } => diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn judge_request() -> whipplescript_kernel::coerce::CoerceRequest {
    whipplescript_kernel::coerce::CoerceRequest {
        function_name: String::new(),
        arguments_json: String::new(),
        output_type: String::new(),
        generated_coerce_source_hash: String::new(),
        input_schema_hash: String::new(),
        output_schema_hash: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Gauge specs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum JudgeSpec {
    Exec(String),
    Labels(String),
    Prompt(String),
    Coerce(String),
    Builtin,
}

#[derive(Clone, Debug)]
struct BarSpec {
    /// `Some(field)` for chance-shaped bars (`P(field)`), else the stat name.
    chance_field: Option<String>,
    stat: Option<String>,
    ge: bool,
    threshold: f64,
}

#[derive(Clone, Debug)]
struct GaugeSpec {
    name: String,
    judge: JudgeSpec,
    bar: Option<BarSpec>,
    inputs: Vec<String>,
    /// "Better" direction: quality up, resources down (design note §3);
    /// a user gauge with an `at most` bar is descend-toward-better.
    direction_up: bool,
    builtin: bool,
}

fn bar_from_ir(bar: &whipplescript_parser::IrGaugeBar) -> Option<BarSpec> {
    let threshold = bar.threshold.parse::<f64>().ok()?;
    Some(BarSpec {
        chance_field: (bar.form == "chance").then(|| bar.subject.clone()),
        stat: (bar.form == "stat").then(|| bar.subject.clone()),
        ge: bar.op == ">=",
        threshold,
    })
}

fn collect_gauge_specs(ir: &IrProgram) -> Vec<GaugeSpec> {
    let mut specs: Vec<GaugeSpec> = ir
        .gauges
        .iter()
        .map(|gauge: &IrGauge| {
            let bar = gauge.expect.as_ref().and_then(bar_from_ir);
            let direction_up = bar.as_ref().map(|b| b.ge).unwrap_or(true);
            GaugeSpec {
                name: gauge.name.clone(),
                judge: match gauge.judge_kind.as_str() {
                    "exec" => JudgeSpec::Exec(gauge.judge_target.clone()),
                    "labels" => JudgeSpec::Labels(gauge.judge_target.clone()),
                    "prompt" => JudgeSpec::Prompt(gauge.judge_target.clone()),
                    _ => JudgeSpec::Coerce(gauge.judge_target.clone()),
                },
                bar,
                inputs: gauge.inputs.clone(),
                direction_up,
                builtin: false,
            }
        })
        .collect();
    for name in BUILTIN_GAUGES {
        specs.push(GaugeSpec {
            name: (*name).to_owned(),
            judge: JudgeSpec::Builtin,
            bar: None,
            inputs: Vec::new(),
            direction_up: false,
            builtin: true,
        });
    }
    specs
}

// ---------------------------------------------------------------------------
// Campaign spec (the partition)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct ReachTarget {
    ge: bool,
    threshold: f64,
    raw: String,
}

#[derive(Clone, Debug, Default)]
struct CampaignSpec {
    /// Named (ascending) gauges of the ACTIVE stage, with optional reach
    /// targets. Later `then` stages are recorded (names only — exactly what
    /// the campaign record keeps); v1 executes the first stage, ratchet
    /// execution across stages is a recorded follow-on.
    ascend: Vec<(String, Option<ReachTarget>)>,
    later_stages: Vec<Vec<String>>,
    sacrifice: Vec<String>,
    /// Band overrides in percent of the baseline operating point.
    within_percent: BTreeMap<String, f64>,
    spend_cap_micros: Option<i64>,
    /// Repair mode: restore violated bars, touch nothing else.
    repair: bool,
    /// `Some(name)` when adopted from a declared `campaign` block.
    declared: Option<String>,
}

impl CampaignSpec {
    fn to_json(&self) -> Value {
        json!({
            "ascend": self.ascend.iter().map(|(name, reach)| json!({
                "gauge": name,
                "reach": reach.as_ref().map(|r| json!({
                    "op": if r.ge { ">=" } else { "<=" },
                    "threshold": r.threshold,
                    "raw": r.raw,
                })),
            })).collect::<Vec<_>>(),
            "later_stages": self.later_stages,
            "sacrifice": self.sacrifice,
            "within_percent": self.within_percent,
            "spend_cap_micros": self.spend_cap_micros,
            "repair": self.repair,
            "declared": self.declared,
        })
    }
}

/// Parse a positional improve target: `name`, `name>=0.9`, `name<=800ms`.
/// The CLI sees real operators (only the .whip declaration tokenizer drops
/// them).
fn parse_target(token: &str) -> Result<(String, Option<ReachTarget>), String> {
    for (op, ge) in [(">=", true), ("<=", false)] {
        if let Some((name, rest)) = token.split_once(op) {
            let raw = rest.trim().to_owned();
            let numeric: String = raw
                .chars()
                .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
                .collect();
            let threshold = numeric
                .parse::<f64>()
                .map_err(|_| format!("invalid target `{token}`: `{raw}` is not a number"))?;
            return Ok((
                name.trim().to_owned(),
                Some(ReachTarget { ge, threshold, raw }),
            ));
        }
    }
    Ok((token.to_owned(), None))
}

fn parse_percent(raw: &str) -> Result<f64, String> {
    raw.trim()
        .trim_end_matches('%')
        .parse::<f64>()
        .map_err(|_| format!("invalid band `{raw}` (use e.g. `2%`)"))
}

fn parse_spend_cap(raw: &str) -> Result<i64, String> {
    let cleaned = raw.trim().trim_start_matches('$');
    let dollars = cleaned
        .parse::<f64>()
        .map_err(|_| format!("invalid spend cap `{raw}` (use e.g. `$4`)"))?;
    if !dollars.is_finite() || dollars <= 0.0 {
        return Err(format!(
            "invalid spend cap `{raw}` (must be a positive amount)"
        ));
    }
    Ok((dollars * 1_000_000.0) as i64)
}

struct ImproveArgs {
    spec: CampaignSpec,
    proposer: String,
    provider: String,
    provider_config_paths: Vec<PathBuf>,
    root: Option<String>,
}

fn parse_improve_args(
    args: &[String],
    ir_campaigns: &[(String, CampaignSpec)],
) -> Result<ImproveArgs, String> {
    let mut spec = CampaignSpec::default();
    let mut stages: Vec<Vec<(String, Option<ReachTarget>)>> = vec![Vec::new()];
    let mut proposer = std::env::var("WHIPPLESCRIPT_IMPROVE_PROPOSER").unwrap_or_default();
    let mut provider = "fixture".to_owned();
    let mut provider_config_paths: Vec<PathBuf> = Vec::new();
    let mut root = None;
    let mut declared: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            // --program is consumed here but owned by run_improve's probe
            // scan (it must resolve before compilation, which happens
            // before this parse).
            "--program" => {
                index += 1;
                args.get(index).ok_or("--program requires a path")?;
            }
            "--root" => {
                index += 1;
                root = Some(args.get(index).ok_or("--root requires a path")?.clone());
            }
            "--sacrifice" => {
                index += 1;
                spec.sacrifice.push(
                    args.get(index)
                        .ok_or("--sacrifice requires a gauge name")?
                        .clone(),
                );
            }
            "--within" => {
                index += 1;
                let raw = args.get(index).ok_or("--within requires <gauge>=<band>")?;
                let (gauge, band) = raw
                    .split_once('=')
                    .ok_or_else(|| format!("invalid --within `{raw}` (use `<gauge>=2%`)"))?;
                spec.within_percent
                    .insert(gauge.to_owned(), parse_percent(band)?);
            }
            "--spend-cap" => {
                index += 1;
                spec.spend_cap_micros = Some(parse_spend_cap(
                    args.get(index).ok_or("--spend-cap requires an amount")?,
                )?);
            }
            "--proposer" => {
                index += 1;
                proposer = args
                    .get(index)
                    .ok_or("--proposer requires `fixture` or `native`")?
                    .clone();
            }
            "--provider" => {
                index += 1;
                provider = args
                    .get(index)
                    .ok_or("--provider requires a provider name")?
                    .clone();
            }
            "--provider-config" => {
                index += 1;
                provider_config_paths.push(PathBuf::from(
                    args.get(index).ok_or("--provider-config requires a path")?,
                ));
            }
            "then" => stages.push(Vec::new()),
            flag if flag.starts_with("--") => {
                return Err(format!("unknown improve flag `{flag}`"));
            }
            target => {
                // A single bare positional naming a declared campaign adopts
                // its spec ("adopted from `release_tuning`").
                if stages.len() == 1
                    && stages[0].is_empty()
                    && declared.is_none()
                    && ir_campaigns.iter().any(|(name, _)| name == target)
                {
                    declared = Some(target.to_owned());
                } else {
                    stages
                        .last_mut()
                        .expect("stages always has one entry")
                        .push(parse_target(target)?);
                }
            }
        }
        index += 1;
    }
    if let Some(name) = &declared {
        if stages.iter().any(|stage| !stage.is_empty()) {
            return Err(format!(
                "campaign `{name}` cannot be combined with inline gauge targets"
            ));
        }
        let (_, mut adopted) = ir_campaigns
            .iter()
            .find(|(candidate, _)| candidate == name)
            .cloned()
            .expect("declared campaign resolved above");
        adopted.declared = Some(name.clone());
        // CLI flags refine the declared spec (spend cap, extra sacrifices).
        adopted.sacrifice.extend(spec.sacrifice);
        adopted.within_percent.extend(spec.within_percent);
        adopted.spend_cap_micros = spec.spend_cap_micros.or(adopted.spend_cap_micros);
        spec = adopted;
    } else {
        let mut stages_iter = stages.into_iter().filter(|stage| !stage.is_empty());
        spec.ascend = stages_iter.next().unwrap_or_default();
        spec.later_stages = stages_iter
            .map(|stage| stage.into_iter().map(|(name, _)| name).collect())
            .collect();
        spec.repair = spec.ascend.is_empty() && spec.later_stages.is_empty();
    }
    if proposer.is_empty() {
        proposer = "native".to_owned();
    }
    Ok(ImproveArgs {
        spec,
        proposer,
        provider,
        provider_config_paths,
        root,
    })
}

fn declared_campaign_specs(ir: &IrProgram) -> Result<Vec<(String, CampaignSpec)>, String> {
    ir.campaigns
        .iter()
        .map(|campaign| {
            let mut spec = CampaignSpec {
                ascend: campaign
                    .ascend
                    .iter()
                    .map(|name| (name.clone(), None))
                    .collect(),
                ..Default::default()
            };
            for reach in &campaign.reach {
                // Fallible on purpose: a threshold this converter cannot
                // parse must never silently become 0.0 (a trivially-met
                // target).
                let threshold = reach.threshold.parse::<f64>().map_err(|_| {
                    format!(
                        "campaign `{}`: reach threshold `{}` for `{}` is not a number",
                        campaign.name, reach.threshold, reach.gauge
                    )
                })?;
                spec.ascend.push((
                    reach.gauge.clone(),
                    Some(ReachTarget {
                        ge: reach.op == ">=",
                        threshold,
                        raw: format!("{}{}", reach.threshold, reach.unit.as_deref().unwrap_or("")),
                    }),
                ));
            }
            for guard in &campaign.guard {
                let percent = guard.band_percent.parse::<f64>().map_err(|_| {
                    format!(
                        "campaign `{}`: guard band `{}` for `{}` is not a number",
                        campaign.name, guard.band_percent, guard.gauge
                    )
                })?;
                spec.within_percent.insert(guard.gauge.clone(), percent);
            }
            spec.sacrifice = campaign.sacrifice.clone();
            Ok((campaign.name.clone(), spec))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Scoring: one completed instance -> gauge readings
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct GaugeReading {
    score: f64,
    passed: Option<bool>,
    tags: Vec<String>,
}

#[derive(Clone, Debug)]
struct RunObservation {
    scenario: Option<String>,
    readings: BTreeMap<String, GaugeReading>,
    skipped: Vec<(String, String)>,
}

/// Score every scoreable gauge against one completed instance in `store`.
/// `ambient` restricts to judges that are free and deterministic (exec +
/// labels-with-scenario + builtins); campaign evaluation also scores prompt
/// judges when a native coerce provider is configured.
fn score_instance(
    store: &SqliteStore,
    instance_id: &str,
    specs: &[GaugeSpec],
    scenario: Option<&str>,
    ambient: bool,
) -> RunObservation {
    let mut readings: BTreeMap<String, GaugeReading> = BTreeMap::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let instance = store.get_instance(instance_id).ok().flatten();
    // Consumed facts are still facts the run produced; judges must see
    // the whole record, not the un-consumed residue.
    let facts = store
        .list_facts_including_consumed(instance_id)
        .unwrap_or_default();
    let runs = store.list_runs(instance_id).unwrap_or_default();
    let facts_json: Vec<Value> = facts
        .iter()
        .map(|fact| {
            json!({
                "name": fact.name,
                "key": fact.key,
                "value": serde_json::from_str::<Value>(&fact.value_json)
                    .unwrap_or(Value::Null),
            })
        })
        .collect();
    let judge_input = json!({
        "schema": "whipplescript.judge_input.v0",
        "scenario": scenario,
        "status": instance.as_ref().map(|i| i.status.clone()),
        "input": instance
            .as_ref()
            .and_then(|i| serde_json::from_str::<Value>(&i.input_json).ok())
            .unwrap_or(Value::Null),
        "facts": facts_json,
    });

    // Builtins first (they may feed derived gauges).
    for spec in specs.iter().filter(|spec| spec.builtin) {
        match spec.name.as_str() {
            "std.latency" => {
                if let Some(ms) = total_latency_ms(&runs) {
                    readings.insert(
                        spec.name.clone(),
                        GaugeReading {
                            score: ms,
                            passed: None,
                            tags: Vec::new(),
                        },
                    );
                }
            }
            "std.tokens" => {
                if let Some(tokens) = total_tokens(&runs) {
                    readings.insert(
                        spec.name.clone(),
                        GaugeReading {
                            score: tokens,
                            passed: None,
                            tags: Vec::new(),
                        },
                    );
                }
            }
            // std.spend: no priced observable yet — absent, never fabricated.
            _ => {}
        }
    }

    // Declared gauges as a worklist: a derived gauge scores once every
    // input gauge has scored, so derived-of-derived chains resolve in
    // later passes regardless of declaration order; whatever never becomes
    // ready is skipped honestly.
    let mut pending: Vec<&GaugeSpec> = specs.iter().filter(|spec| !spec.builtin).collect();
    loop {
        let (ready, waiting): (Vec<&GaugeSpec>, Vec<&GaugeSpec>) = pending
            .into_iter()
            .partition(|spec| spec.inputs.iter().all(|input| readings.contains_key(input)));
        pending = waiting;
        if ready.is_empty() {
            break;
        }
        for spec in ready {
            score_one_gauge(
                spec,
                &judge_input,
                scenario,
                ambient,
                &mut readings,
                &mut skipped,
            );
        }
    }
    for spec in pending {
        skipped.push((spec.name.clone(), "inputs unscored on this run".to_owned()));
    }
    RunObservation {
        scenario: scenario.map(str::to_owned),
        readings,
        skipped,
    }
}

/// Score one non-builtin gauge whose inputs (if any) are all present in
/// `readings`.
fn score_one_gauge(
    spec: &GaugeSpec,
    judge_input: &Value,
    scenario: Option<&str>,
    ambient: bool,
    readings: &mut BTreeMap<String, GaugeReading>,
    skipped: &mut Vec<(String, String)>,
) {
    {
        let mut input = judge_input.clone();
        if !spec.inputs.is_empty() {
            let scores: BTreeMap<&String, f64> = spec
                .inputs
                .iter()
                .map(|name| (name, readings[name].score))
                .collect();
            input["inputs"] = json!(scores);
        }
        match &spec.judge {
            JudgeSpec::Exec(command) => match run_exec_judge(command, &input, spec) {
                Ok(reading) => {
                    readings.insert(spec.name.clone(), reading);
                }
                Err(reason) => skipped.push((spec.name.clone(), reason)),
            },
            JudgeSpec::Labels(source) => match lookup_label(source, scenario, spec) {
                Ok(Some(reading)) => {
                    readings.insert(spec.name.clone(), reading);
                }
                Ok(None) => skipped.push((spec.name.clone(), "no label for this run".to_owned())),
                Err(reason) => skipped.push((spec.name.clone(), reason)),
            },
            JudgeSpec::Prompt(template) => {
                if ambient {
                    skipped.push((
                        spec.name.clone(),
                        "prompt judges are scored during campaigns/settle, not ambiently (v1)"
                            .to_owned(),
                    ));
                } else {
                    match run_prompt_judge(template, &input, spec) {
                        Ok(reading) => {
                            readings.insert(spec.name.clone(), reading);
                        }
                        Err(reason) => skipped.push((spec.name.clone(), reason)),
                    }
                }
            }
            JudgeSpec::Coerce(target) => skipped.push((
                spec.name.clone(),
                format!("coerce judge `{target}` is not yet scoreable (v1)"),
            )),
            JudgeSpec::Builtin => {}
        }
    }
}

fn parse_store_timestamp(raw: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f"))
        .ok()
        .or_else(|| {
            // Other store columns carry RFC3339; tolerate it here so a
            // storage-format change never silently vanishes std.latency.
            chrono::DateTime::parse_from_rfc3339(raw)
                .map(|instant| instant.naive_utc())
                .ok()
        })
}

fn total_latency_ms(runs: &[whipplescript_store::RunView]) -> Option<f64> {
    let mut total = 0.0;
    let mut any = false;
    for run in runs {
        let (Some(started), Some(completed)) = (
            parse_store_timestamp(&run.started_at),
            run.completed_at.as_deref().and_then(parse_store_timestamp),
        ) else {
            continue;
        };
        total += (completed - started).num_milliseconds().max(0) as f64;
        any = true;
    }
    any.then_some(total)
}

fn total_tokens(runs: &[whipplescript_store::RunView]) -> Option<f64> {
    let mut total = 0.0;
    let mut any = false;
    for run in runs {
        let Ok(metadata) = serde_json::from_str::<Value>(&run.metadata_json) else {
            continue;
        };
        // One usage object per run; prefer the provider's own total, else
        // input+output — never sum overlapping fields.
        let usage = metadata.get("usage").or_else(|| metadata.get("usage_json"));
        if let Some(usage) = usage {
            if let Some(count) = usage.get("total_tokens").and_then(Value::as_f64) {
                total += count;
                any = true;
            } else {
                for tokens_key in ["input_tokens", "output_tokens"] {
                    if let Some(count) = usage.get(tokens_key).and_then(Value::as_f64) {
                        total += count;
                        any = true;
                    }
                }
            }
        }
    }
    any.then_some(total)
}

fn reading_from_judge_output(output: &Value, spec: &GaugeSpec) -> Result<GaugeReading, String> {
    let passed = if let Some(bar) = &spec.bar {
        if let Some(field) = &bar.chance_field {
            Some(
                output
                    .get(field)
                    .and_then(Value::as_bool)
                    .ok_or_else(|| format!("judge output has no boolean field `{field}`"))?,
            )
        } else {
            output.get("passed").and_then(Value::as_bool)
        }
    } else {
        output.get("passed").and_then(Value::as_bool)
    };
    let score = output
        .get("score")
        .and_then(Value::as_f64)
        .or(passed.map(|p| if p { 1.0 } else { 0.0 }))
        .ok_or("judge output has neither `score` nor a bar verdict")?;
    Ok(GaugeReading {
        score,
        passed,
        tags: Vec::new(),
    })
}

/// Run a deterministic exec judge: the shipped det-validation pattern with
/// the run context on stdin and a JSON object on stdout. Governed by the
/// same operator-config-only allowlist as `exec` effects
/// (`WHIPPLESCRIPT_EXEC_ALLOW`): source declares, config grants.
fn run_exec_judge(command: &str, input: &Value, spec: &GaugeSpec) -> Result<GaugeReading, String> {
    if !crate::exec_command_granted(command) {
        return Err(format!(
            "exec judge `{command}` is not granted (WHIPPLESCRIPT_EXEC_ALLOW)"
        ));
    }
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn exec judge: {error}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        if let Err(error) = stdin.write_all(input.to_string().as_bytes()) {
            // A judge that decides without reading stdin can exit before the
            // write lands; EPIPE means "input not consumed", not a failed
            // judge — its own output and exit code decide.
            if error.kind() != std::io::ErrorKind::BrokenPipe {
                let _ = child.kill();
                return Err(format!("failed to write judge input: {error}"));
            }
        }
    }
    // Close stdin so a judge reading to EOF can proceed (wait_with_output
    // would do this implicitly; the bounded wait below runs first).
    drop(child.stdin.take());
    // Bounded wait: a wedged judge must never hang the dev loop or a
    // campaign; kill and skip (the gauge lands in `skipped`, tagged).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err("exec judge timed out after 60s".to_owned());
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(format!("exec judge failed: {error}")),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("exec judge failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "exec judge exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let parsed: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("exec judge emitted invalid JSON: {error}"))?;
    reading_from_judge_output(&parsed, spec)
}

/// Wholesale anchors: a user-owned JSON label file mapping scenario name to
/// `{"passed": bool}` / `{"score": n}`. Labels are trusted by declaration
/// and parsed once per process (they cannot change mid-campaign
/// meaningfully; a campaign re-reads by restarting).
fn lookup_label(
    source: &str,
    scenario: Option<&str>,
    spec: &GaugeSpec,
) -> Result<Option<GaugeReading>, String> {
    static LABELS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, Value>>> =
        std::sync::OnceLock::new();
    let Some(scenario) = scenario else {
        return Ok(None);
    };
    let cache = LABELS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()));
    let mut cache = cache.lock().map_err(|_| "labels cache poisoned")?;
    if !cache.contains_key(source) {
        let raw = std::fs::read_to_string(source)
            .map_err(|error| format!("labels source `{source}`: {error}"))?;
        let labels: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("labels source `{source}` is not JSON: {error}"))?;
        cache.insert(source.to_owned(), labels);
    }
    let labels = cache.get(source).expect("inserted above");
    let Some(entry) = labels.get(scenario) else {
        return Ok(None);
    };
    let mut reading = reading_from_judge_output(entry, spec)?;
    reading.tags.push("wholesale-anchor".to_owned());
    Ok(Some(reading))
}

/// Evidence provenance for the scorer column — ONE mapping, shared by the
/// campaign (regen) and ambient (live) recorders so the same instrument
/// never splits into two scorer identities.
fn scorer_label(judge: &JudgeSpec) -> String {
    match judge {
        JudgeSpec::Exec(command) => format!("exec:{command}"),
        JudgeSpec::Labels(source) => format!("labels:{source}"),
        JudgeSpec::Prompt(_) => "prompt".to_owned(),
        JudgeSpec::Coerce(name) => format!("coerce:{name}"),
        JudgeSpec::Builtin => "builtin".to_owned(),
    }
}

/// One structured native-coerce turn (the judge/proposer transport shell):
/// prompt + object schema in, parsed value + total token usage out.
fn native_coerce_turn(
    purpose: &str,
    prompt: String,
    schema: Value,
    schema_name: &str,
    codex_label: &str,
) -> Result<(Value, i64), String> {
    let config = crate::coerce_runtime::resolve_native_coerce_config()
        .map_err(|error| format!("{purpose} provider: {error}"))?
        .ok_or_else(|| {
            format!(
                "{purpose} needs a native coerce provider (set \
                 WHIPPLESCRIPT_COERCE_PROVIDER or run `whip auth`)"
            )
        })?;
    let transport = crate::coerce_runtime::UreqCoerceTransport::new(config.timeout);
    let client = whipplescript_kernel::coerce_native::NativeCoerceClient {
        provider: config.provider,
        base_url: config.base_url.clone(),
        api_key: config.api_key.clone(),
        model: config.model.clone(),
        prompt,
        output_schema: schema,
        wrapped: false,
        schema_name: schema_name.to_owned(),
        max_tokens: config.max_tokens,
        codex: config
            .codex_account_id
            .as_ref()
            .map(|account| (account.clone(), codex_label.to_owned())),
        idempotency_key: String::new(),
        transport: &transport,
    };
    let result = whipplescript_kernel::coerce::CoerceClient::coerce(&client, &judge_request());
    if !matches!(
        result.status,
        whipplescript_kernel::coerce::CoerceStatus::Succeeded
    ) {
        return Err(format!("{purpose} failed: {}", result.summary));
    }
    let usage: Value = serde_json::from_str(&result.usage_json).unwrap_or(Value::Null);
    let tokens = ["input_tokens", "output_tokens", "total_tokens"]
        .iter()
        .filter_map(|key| usage.get(key).and_then(Value::as_i64))
        .sum();
    let value: Value = result
        .value_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .ok_or_else(|| format!("{purpose} returned no value"))?;
    Ok((value, tokens))
}

/// LLM prompt judge via the native coerce path; requires a configured
/// provider (WHIPPLESCRIPT_COERCE_PROVIDER / `whip auth`).
fn run_prompt_judge(
    template: &str,
    input: &Value,
    spec: &GaugeSpec,
) -> Result<GaugeReading, String> {
    let prompt = format!(
        "{template}\n\nJudge the following run record. Respond with the JSON schema \
         provided.\n\n{input}"
    );
    let schema = json!({
        "type": "object",
        "properties": {
            "passed": {"type": "boolean"},
            "score": {"type": "number"},
            "rationale": {"type": "string"},
        },
        "required": ["passed", "score", "rationale"],
        "additionalProperties": false,
    });
    let (value, _tokens) = native_coerce_turn(
        "prompt judge",
        prompt,
        schema,
        "GaugeJudgeVerdict",
        &format!("improve-judge-{}", spec.name),
    )?;
    let mut reading = reading_from_judge_output(&value, spec)?;
    reading.tags.push("judge-unanchored".to_owned());
    Ok(reading)
}

// ---------------------------------------------------------------------------
// Evaluation: run a program over scenarios in disposable stores
// ---------------------------------------------------------------------------

fn eval_scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("whip-improve-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Regenerate one scenario under `program_path` in a disposable store and
/// score it. The temp store is the v1 storage-plane containment: a
/// counterfactual run's writes land nowhere near the workspace store.
#[allow(clippy::too_many_arguments)]
fn evaluate_scenario(
    program_path: &str,
    root: Option<&str>,
    provider: &str,
    provider_config_paths: &[PathBuf],
    scenario: &ScenarioRow,
    specs: &[GaugeSpec],
    ir: &IrProgram,
    seq: &mut usize,
) -> Result<RunObservation, String> {
    *seq += 1;
    let store_path = eval_scratch_dir().join(format!("eval-{seq}.sqlite"));
    let _ = std::fs::remove_file(&store_path);
    let eval_options = CliOptions {
        command: Some("improve".to_owned()),
        args: Vec::new(),
        store_path: store_path.clone(),
        json: true,
        input_json: Some(scenario.input_json.clone()),
    };
    let started = crate::start_workflow_instance(
        program_path,
        root,
        None,
        Some(scenario.input_json.as_str()),
        &eval_options,
    )
    .map_err(|_| {
        format!(
            "failed to start evaluation instance for `{}`",
            scenario.name
        )
    })?;
    for _ in 0..16 {
        let step_report = crate::step_instance(
            &store_path,
            &started.instance_id,
            ir,
            Some(Path::new(program_path)),
            None,
        )
        .map_err(|error| format!("evaluation step failed: {error:?}"))?;
        let worker_report = crate::run_worker_once(
            &store_path,
            &crate::WorkerOptions {
                instance_id: started.instance_id.clone(),
                provider: provider.to_owned(),
                exec_profile: crate::ExecProfile::from_env(),
                script_manifest_path: None,
                package_lock_path: None,
                outcome: crate::FixtureOutcome::default(),
                variant: None,
                program_path: Some(PathBuf::from(program_path)),
                root: root.map(str::to_owned),
                provider_config_paths: provider_config_paths.to_vec(),
                max_child_iterations: 8,
                agent_outcomes: BTreeMap::new(),
                coerce_outputs: BTreeMap::new(),
                virtual_now: None,
                work_unit_root: None,
            },
        )
        .map_err(|error| format!("evaluation worker failed: {error:?}"))?;
        if crate::drive_pass_idle(&step_report, &worker_report) {
            break;
        }
    }
    let store = SqliteStore::open(&store_path)
        .map_err(|error| format!("failed to reopen evaluation store: {error:?}"))?;
    let observation = score_instance(
        &store,
        &started.instance_id,
        specs,
        Some(&scenario.name),
        false,
    );
    let _ = std::fs::remove_file(&store_path);
    Ok(observation)
}

// ---------------------------------------------------------------------------
// Aggregation, bands, dominance
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct GaugeAggregate {
    scores: Vec<f64>,
    passes: Vec<bool>,
}

impl GaugeAggregate {
    fn n(&self) -> usize {
        self.scores.len()
    }
    fn mean(&self) -> Option<f64> {
        (!self.scores.is_empty())
            .then(|| self.scores.iter().sum::<f64>() / self.scores.len() as f64)
    }
    fn pass_rate(&self) -> Option<f64> {
        (!self.passes.is_empty())
            .then(|| self.passes.iter().filter(|p| **p).count() as f64 / self.passes.len() as f64)
    }
    fn quantile(&self, q: f64) -> Option<f64> {
        if self.scores.is_empty() {
            return None;
        }
        let mut sorted = self.scores.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let index = ((sorted.len() - 1) as f64 * q).round() as usize;
        Some(sorted[index.min(sorted.len() - 1)])
    }
    /// The operating point compared by the dominance check: pass rate when
    /// the gauge is chance-shaped, mean score otherwise.
    fn operating_point(&self) -> Option<f64> {
        self.pass_rate().or_else(|| self.mean())
    }
}

fn aggregate(observations: &[RunObservation], gauge: &str) -> GaugeAggregate {
    let mut aggregate = GaugeAggregate::default();
    for observation in observations {
        if let Some(reading) = observation.readings.get(gauge) {
            aggregate.scores.push(reading.score);
            if let Some(passed) = reading.passed {
                aggregate.passes.push(passed);
            }
        }
    }
    aggregate
}

fn bar_stat(aggregate: &GaugeAggregate, bar: &BarSpec) -> Option<f64> {
    if bar.chance_field.is_some() {
        return aggregate.pass_rate();
    }
    match bar.stat.as_deref() {
        Some("mean") | None => aggregate.mean(),
        Some(stat) => {
            let quantile = stat.strip_prefix('p')?.parse::<f64>().ok()? / 100.0;
            aggregate.quantile(quantile)
        }
    }
}

fn bar_met(aggregate: &GaugeAggregate, bar: &BarSpec) -> Option<bool> {
    let value = bar_stat(aggregate, bar)?;
    Some(if bar.ge {
        value >= bar.threshold
    } else {
        value <= bar.threshold
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Delta {
    Better,
    InBand,
    Worse,
    Unmeasured,
}

/// The indifference band around the baseline operating point: the gauge's
/// noise floor by default (a pooled-SE minimal detectable effect), relative
/// bands for resource gauges (their noise floor degenerates toward zero),
/// `--within` / campaign `guard` overrides in percent of baseline.
fn indifference_band(
    spec: &GaugeSpec,
    base: &GaugeAggregate,
    cand: &GaugeAggregate,
    within_percent: Option<f64>,
    baseline_point: f64,
) -> f64 {
    if let Some(percent) = within_percent {
        return (baseline_point.abs() * percent / 100.0).max(f64::EPSILON);
    }
    if spec.builtin {
        return (baseline_point.abs() * RESOURCE_BAND_PERCENT / 100.0).max(f64::EPSILON);
    }
    // Noise floor: 1.96 * pooled standard error over the compared samples.
    let se = |aggregate: &GaugeAggregate| -> f64 {
        let n = aggregate.n().max(1) as f64;
        if let Some(rate) = aggregate.pass_rate() {
            (rate * (1.0 - rate) / n).sqrt()
        } else if let Some(mean) = aggregate.mean() {
            let variance = aggregate
                .scores
                .iter()
                .map(|score| (score - mean).powi(2))
                .sum::<f64>()
                / n;
            (variance / n).sqrt()
        } else {
            0.0
        }
    };
    let pooled = (se(base).powi(2) + se(cand).powi(2)).sqrt();
    (1.96 * pooled).max(QUALITY_BAND_FLOOR)
}

fn delta_verdict(
    spec: &GaugeSpec,
    base: &GaugeAggregate,
    cand: &GaugeAggregate,
    within_percent: Option<f64>,
) -> (Delta, f64, f64) {
    let (Some(base_point), Some(cand_point)) = (base.operating_point(), cand.operating_point())
    else {
        return (Delta::Unmeasured, 0.0, 0.0);
    };
    let band = indifference_band(spec, base, cand, within_percent, base_point);
    let signed = if spec.direction_up {
        cand_point - base_point
    } else {
        base_point - cand_point
    };
    let verdict = if signed > band {
        Delta::Better
    } else if signed < -band {
        Delta::Worse
    } else {
        Delta::InBand
    };
    (verdict, cand_point - base_point, band)
}

#[derive(Clone, Debug)]
struct GaugeVerdictLine {
    gauge: String,
    role: &'static str,
    delta: Delta,
    baseline: Option<f64>,
    candidate: Option<f64>,
    band: f64,
    bar_met: Option<bool>,
    reach_met: Option<bool>,
}

#[derive(Clone, Debug)]
struct CandidateVerdict {
    lines: Vec<GaugeVerdictLine>,
    proposable: bool,
    tradeoff: bool,
    reasons: Vec<String>,
}

/// The dominance invariant, exactly as modeled: accept only if at least one
/// ascend gauge improved, no ascend gauge regressed, every guarded gauge
/// held within its band, every declared bar is met, and sacrificed gauges
/// move freely (the evidence card says so). Reach targets ratchet: a target
/// the BASELINE already met is a hard bound the candidate may not drop
/// below; an unmet target never blocks genuine progress toward it. Repair
/// mode proposes only a candidate that RESTORES a bar the baseline
/// violated, without moving anything else beyond band.
fn dominance_verdict(
    specs: &[GaugeSpec],
    campaign: &CampaignSpec,
    base: &[RunObservation],
    cand: &[RunObservation],
) -> CandidateVerdict {
    let ascend_names: BTreeMap<&str, Option<&ReachTarget>> = campaign
        .ascend
        .iter()
        .map(|(name, reach)| (name.as_str(), reach.as_ref()))
        .collect();
    let sacrificed: BTreeSet<&str> = campaign.sacrifice.iter().map(String::as_str).collect();
    let mut lines = Vec::new();
    let mut reasons = Vec::new();
    let mut focus_up = false;
    let mut focus_down = false;
    let mut guard_broken = false;
    let mut bar_violated = false;
    let mut bar_restored = false;
    for spec in specs {
        let base_aggregate = aggregate(base, &spec.name);
        let cand_aggregate = aggregate(cand, &spec.name);
        if base_aggregate.n() == 0 && cand_aggregate.n() == 0 {
            continue;
        }
        let within = campaign.within_percent.get(&spec.name).copied();
        let (delta, _, band) = delta_verdict(spec, &base_aggregate, &cand_aggregate, within);
        let bar_status = spec
            .bar
            .as_ref()
            .and_then(|bar| bar_met(&cand_aggregate, bar));
        let baseline_bar_status = spec
            .bar
            .as_ref()
            .and_then(|bar| bar_met(&base_aggregate, bar));
        if bar_status == Some(false) {
            bar_violated = true;
            reasons.push(format!("`{}` violates its declared bar", spec.name));
        }
        if baseline_bar_status == Some(false) && bar_status == Some(true) {
            bar_restored = true;
        }
        let reach = ascend_names.get(spec.name.as_str()).copied().flatten();
        let reach_met = reach.and_then(|reach| {
            cand_aggregate.operating_point().map(|point| {
                if reach.ge {
                    point >= reach.threshold
                } else {
                    point <= reach.threshold
                }
            })
        });
        let role;
        if ascend_names.contains_key(spec.name.as_str()) {
            role = "ascend";
            match delta {
                Delta::Better => focus_up = true,
                Delta::Worse => {
                    focus_down = true;
                    reasons.push(format!("`{}` regressed (its own focus)", spec.name));
                }
                _ => {}
            }
            // Ratchet: a reach target the baseline had already achieved is
            // a hard bound; dropping back below it is a refusal even if the
            // movement sits inside the band.
            if let Some(reach) = reach {
                let baseline_met = base_aggregate.operating_point().map(|point| {
                    if reach.ge {
                        point >= reach.threshold
                    } else {
                        point <= reach.threshold
                    }
                });
                if baseline_met == Some(true) && reach_met == Some(false) {
                    bar_violated = true;
                    reasons.push(format!(
                        "`{}` dropped below its achieved reach target (ratchet)",
                        spec.name
                    ));
                }
            }
        } else if sacrificed.contains(spec.name.as_str()) {
            role = "sacrifice";
        } else {
            role = "guard";
            if delta == Delta::Worse {
                guard_broken = true;
                reasons.push(format!(
                    "`{}` regressed beyond its indifference band and was not sacrificed",
                    spec.name
                ));
            }
            // Fail closed: a guarded gauge that was measured at baseline but
            // became unmeasurable on the candidate cannot certify
            // non-regression — refuse rather than silently pass.
            if delta == Delta::Unmeasured && base_aggregate.n() > 0 && cand_aggregate.n() == 0 {
                guard_broken = true;
                reasons.push(format!(
                    "`{}` became unmeasurable on the candidate (guarded gauges fail closed)",
                    spec.name
                ));
            }
        }
        lines.push(GaugeVerdictLine {
            gauge: spec.name.clone(),
            role,
            delta,
            baseline: base_aggregate.operating_point(),
            candidate: cand_aggregate.operating_point(),
            band,
            bar_met: bar_status,
            reach_met,
        });
    }
    // Repair mode: proposable iff a bar the BASELINE violated is restored
    // and nothing moved beyond band — a no-op on a healthy program repairs
    // nothing and is refused.
    let proposable = if campaign.repair {
        bar_restored && !bar_violated && !guard_broken
    } else {
        focus_up && !focus_down && !guard_broken && !bar_violated
    };
    let tradeoff = focus_up && guard_broken && !bar_violated && !focus_down;
    CandidateVerdict {
        lines,
        proposable,
        tradeoff,
        reasons,
    }
}

// ---------------------------------------------------------------------------
// Holdout sealing
// ---------------------------------------------------------------------------

fn seal_scenarios<'a>(
    campaign_id: &str,
    scenarios: &'a [ScenarioRow],
) -> (Vec<&'a ScenarioRow>, Vec<&'a ScenarioRow>, bool) {
    let eligible: Vec<&ScenarioRow> = scenarios.iter().filter(|s| !s.retired).collect();
    if eligible.len() < MIN_SCENARIOS_FOR_SEALING {
        return (eligible, Vec::new(), false);
    }
    let sealed_count = ((eligible.len() as f64 * SEALED_FRACTION).ceil() as usize)
        .max(SEALED_FLOOR)
        .min(eligible.len().saturating_sub(2));
    let mut ranked: Vec<(&ScenarioRow, String)> = eligible
        .iter()
        .map(|scenario| {
            let digest = Sha256::digest(format!("{campaign_id}|{}", scenario.name).as_bytes());
            (*scenario, format!("{digest:x}"))
        })
        .collect();
    ranked.sort_by(|a, b| a.1.cmp(&b.1));
    let sealed: Vec<&ScenarioRow> = ranked
        .iter()
        .take(sealed_count)
        .map(|(scenario, _)| *scenario)
        .collect();
    let sealed_names: BTreeSet<&str> = sealed.iter().map(|s| s.name.as_str()).collect();
    let open: Vec<&ScenarioRow> = eligible
        .into_iter()
        .filter(|scenario| !sealed_names.contains(scenario.name.as_str()))
        .collect();
    (open, sealed, true)
}

// ---------------------------------------------------------------------------
// The proposer
// ---------------------------------------------------------------------------

trait Proposer {
    fn propose(&mut self, reflection: &str) -> Result<Option<Proposal>, String>;
    fn name(&self) -> &'static str;
}

struct Proposal {
    source: String,
    rationale: String,
    /// Provider token usage of the proposing turn (0 for fixture), recorded
    /// as a `campaign.spend` event.
    tokens: i64,
}

/// Deterministic test/dev proposer: colon-separated candidate source paths
/// in WHIPPLESCRIPT_IMPROVE_PROPOSALS, consumed one per round.
struct FixtureProposer {
    remaining: Vec<String>,
}

impl FixtureProposer {
    fn from_env() -> Self {
        let remaining = std::env::var("WHIPPLESCRIPT_IMPROVE_PROPOSALS")
            .unwrap_or_default()
            .split(':')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect();
        Self { remaining }
    }
}

impl Proposer for FixtureProposer {
    fn propose(&mut self, _reflection: &str) -> Result<Option<Proposal>, String> {
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let path = self.remaining.remove(0);
        let source = std::fs::read_to_string(&path)
            .map_err(|error| format!("fixture proposal `{path}`: {error}"))?;
        Ok(Some(Proposal {
            source,
            rationale: format!("fixture proposal from {path}"),
            tokens: 0,
        }))
    }
    fn name(&self) -> &'static str {
        "fixture"
    }
}

/// The reflective proposer: one structured native-coerce turn fed the
/// reflection material (never sealed traces — the caller builds the
/// reflection holdout-blind). Propose-don't-apply: its output is a
/// candidate, never a write.
struct NativeProposer;

impl Proposer for NativeProposer {
    fn propose(&mut self, reflection: &str) -> Result<Option<Proposal>, String> {
        let schema = json!({
            "type": "object",
            "properties": {
                "rationale": {"type": "string"},
                "source": {"type": "string"},
            },
            "required": ["rationale", "source"],
            "additionalProperties": false,
        });
        let (value, tokens) = native_coerce_turn(
            "the native proposer",
            reflection.to_owned(),
            schema,
            "ImproveProposal",
            "improve-proposer",
        )
        .map_err(|error| format!("{error}, or use --proposer fixture"))?;
        let source = value
            .get("source")
            .and_then(Value::as_str)
            .ok_or("proposer returned no source")?
            .to_owned();
        let rationale = value
            .get("rationale")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Ok(Some(Proposal {
            source,
            rationale,
            tokens,
        }))
    }
    fn name(&self) -> &'static str {
        "native"
    }
}

/// The reflection material: program source, the campaign partition, and
/// open-scenario evidence. Sealed scenarios appear as AGGREGATE pass rates
/// only — never traces, contents, or judge rationales
/// (`improve-holdout.maude`).
#[allow(clippy::too_many_arguments)]
fn build_reflection(
    source: &str,
    campaign: &CampaignSpec,
    specs: &[GaugeSpec],
    baseline_open: &[RunObservation],
    baseline_sealed: &[RunObservation],
    open_scenarios: &[&ScenarioRow],
    prior_failures: &[String],
    sealed_engaged: bool,
) -> String {
    let mut reflection = String::new();
    reflection.push_str(
        "You are the improve proposer for a WhippleScript workflow. Propose ONE \
         targeted revision of the program below. Return the COMPLETE revised \
         program source. Improve the ascend gauges without regressing any \
         guarded gauge; declared bars are hard constraints.\n\n",
    );
    reflection.push_str(&format!("## Campaign\n{}\n\n", campaign.to_json()));
    reflection.push_str("## Gauge evidence (open scenarios)\n");
    for spec in specs {
        let aggregate = aggregate(baseline_open, &spec.name);
        if aggregate.n() == 0 {
            continue;
        }
        reflection.push_str(&format!(
            "- {}: operating point {:.4} over N={}\n",
            spec.name,
            aggregate.operating_point().unwrap_or(0.0),
            aggregate.n()
        ));
    }
    if sealed_engaged {
        let mut sealed_line =
            String::from("\n## Sealed holdout (aggregates only; traces withheld)\n");
        for spec in specs {
            let aggregate = aggregate(baseline_sealed, &spec.name);
            if let Some(rate) = aggregate.pass_rate() {
                sealed_line.push_str(&format!(
                    "- {}: pass rate {:.4} over N={}\n",
                    spec.name,
                    rate,
                    aggregate.n()
                ));
            }
        }
        reflection.push_str(&sealed_line);
    }
    reflection.push_str("\n## Worst open scenarios\n");
    for observation in baseline_open {
        let failing: Vec<&str> = observation
            .readings
            .iter()
            .filter(|(_, reading)| reading.passed == Some(false))
            .map(|(gauge, _)| gauge.as_str())
            .collect();
        if let (Some(scenario), false) = (&observation.scenario, failing.is_empty()) {
            reflection.push_str(&format!(
                "- scenario `{scenario}` fails: {}\n",
                failing.join(", ")
            ));
            if let Some(row) = open_scenarios
                .iter()
                .find(|row| Some(&row.name) == observation.scenario.as_ref())
            {
                reflection.push_str(&format!("  input: {}\n", row.input_json));
            }
        }
    }
    if !prior_failures.is_empty() {
        reflection.push_str("\n## Prior candidates that were refused\n");
        for failure in prior_failures {
            reflection.push_str(&format!("- {failure}\n"));
        }
    }
    reflection.push_str(&format!("\n## Program\n```whip\n{source}\n```\n"));
    reflection
}

// ---------------------------------------------------------------------------
// Evidence recording
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn record_observations(
    store: &mut ImproveStore,
    observations: &[RunObservation],
    specs: &[GaugeSpec],
    execution_mode: &str,
    hash: &str,
    campaign_id: Option<&str>,
    candidate_id: Option<&str>,
    extra_tags: &[String],
) {
    for observation in observations {
        for (gauge, reading) in &observation.readings {
            let scorer = specs
                .iter()
                .find(|spec| &spec.name == gauge)
                .map(|spec| scorer_label(&spec.judge))
                .unwrap_or_else(|| "unknown".to_owned());
            let mut tags = reading.tags.clone();
            tags.extend(extra_tags.iter().cloned());
            let _ = store.record_evidence(NewEvidence {
                gauge,
                score: reading.score,
                passed: reading.passed,
                instance_id: None,
                program_hash: Some(hash),
                branch_ref: None,
                execution_mode,
                scorer: &scorer,
                scenario: observation.scenario.as_deref(),
                campaign_id,
                candidate_id,
                cost_micros: 0,
                tags,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// whip improve
// ---------------------------------------------------------------------------

fn resolve_program_path(explicit: Option<String>) -> Result<String, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(".")
        .map_err(|error| format!("failed to scan for a program: {error}"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "whip"))
        .collect();
    candidates.sort();
    match candidates.len() {
        0 => Err("no .whip program found; pass --program <path>".to_owned()),
        1 => Ok(candidates.remove(0).to_string_lossy().into_owned()),
        _ => Err(format!(
            "multiple .whip programs found ({}); pass --program <path>",
            candidates
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub(crate) fn improve_command(options: &CliOptions) -> ExitCode {
    match run_improve(options) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run_improve(options: &CliOptions) -> Result<ExitCode, String> {
    // Compile first (gauge declarations live in the program).
    let (probe_path, probe_root) = {
        // Peek --program/--root before full arg parsing so declared
        // campaigns can inform positional-arg interpretation and a
        // multi-workflow program compiles under its root.
        let mut path = None;
        let mut probe_root = None;
        let mut iter = options.args.iter();
        while let Some(arg) = iter.next() {
            if arg == "--program" {
                path = iter.next().cloned();
            } else if arg == "--root" {
                probe_root = iter.next().cloned();
            }
        }
        (resolve_program_path(path)?, probe_root)
    };
    let (source, ir) = crate::compile_source_path_with_root(&probe_path, probe_root.as_deref())
        .map_err(|error| {
            format!(
                "`{probe_path}` does not compile: {}",
                compile_failure_summary(&error)
            )
        })?;
    let declared = declared_campaign_specs(&ir)?;
    let args = parse_improve_args(&options.args, &declared)?;
    let program_path = probe_path;
    let specs = collect_gauge_specs(&ir);
    if specs.iter().all(|spec| spec.builtin) && !args.spec.repair {
        return Err(
            "no gauges declared; declare `gauge <name> { judge via ... }` before improving"
                .to_owned(),
        );
    }
    for (name, _) in &args.spec.ascend {
        if !specs.iter().any(|spec| &spec.name == name) {
            return Err(format!(
                "unknown gauge `{name}` (declared gauges: {})",
                specs
                    .iter()
                    .map(|spec| spec.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let baseline_hash = program_hash(&source);
    let mut store = open_improve_store()?;
    let scenarios = store
        .list_scenarios()
        .map_err(|error| format!("failed to list scenarios: {error:?}"))?;
    if scenarios.is_empty() {
        return Err(
            "no pinned scenarios; pin at least one run first (`whip pin <instance> --as <name>`)"
                .to_owned(),
        );
    }
    let campaign_id = store
        .open_campaign(&json!({
            "spec": args.spec.to_json(),
            "program": program_path,
            "baseline_hash": baseline_hash,
            "proposer": args.proposer,
        }))
        .map_err(|error| format!("failed to open campaign: {error:?}"))?;
    let (open, sealed, sealing_engaged) = seal_scenarios(&campaign_id, &scenarios);
    let unheld_out = !sealing_engaged;
    // v1 storage-plane containment: for the rest of this process every
    // workspace-scoped side store (coordination leases/counters/ledgers,
    // backlog items, harness content) resolves into the eval scratch, so a
    // counterfactual run's writes land nowhere near the workspace stores.
    std::env::set_var(
        "WHIPPLESCRIPT_COORDINATION_STORE",
        eval_scratch_dir().join("coordination.sqlite"),
    );
    std::env::set_var(
        "WHIPPLESCRIPT_ITEMS_STORE",
        eval_scratch_dir().join("items.sqlite"),
    );
    std::env::set_var(
        "WHIPPLESCRIPT_CONTENT_STORE",
        eval_scratch_dir().join("content.sqlite"),
    );
    let mut campaign_tags: Vec<String> = Vec::new();
    if unheld_out {
        campaign_tags.push("unheld-out".to_owned());
    }
    if args.provider == "fixture" {
        // Fixture-evaluated evidence must never pass for model behavior.
        campaign_tags.push("fixture-provider".to_owned());
    }
    let outcome = (|store: &mut ImproveStore| -> Result<(Vec<Value>, bool, usize), String> {
        store
            .append_campaign_event(
                &campaign_id,
                "campaign.sealed",
                &json!({
                    "sealed": sealed.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
                    "open": open.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
                    "unheld_out": unheld_out,
                }),
            )
            .map_err(|error| format!("failed to record sealing: {error:?}"))?;

        // Baseline evaluation (paired regeneration: same scenarios, both arms).
        let mut seq = 0usize;
        let evaluate_all = |path: &str,
                            ir: &IrProgram,
                            rows: &[&ScenarioRow],
                            seq: &mut usize|
         -> Result<Vec<RunObservation>, String> {
            rows.iter()
                .map(|scenario| {
                    evaluate_scenario(
                        path,
                        args.root.as_deref(),
                        &args.provider,
                        &args.provider_config_paths,
                        scenario,
                        &specs,
                        ir,
                        seq,
                    )
                })
                .collect()
        };
        let baseline_open = evaluate_all(&program_path, &ir, &open, &mut seq)?;
        let baseline_sealed = evaluate_all(&program_path, &ir, &sealed, &mut seq)?;
        let mut baseline_tags = campaign_tags.clone();
        baseline_tags.push("baseline".to_owned());
        record_observations(
            store,
            &baseline_open,
            &specs,
            "regen",
            &baseline_hash,
            Some(&campaign_id),
            Some("baseline"),
            &baseline_tags,
        );
        record_observations(
            store,
            &baseline_sealed,
            &specs,
            "regen",
            &baseline_hash,
            Some(&campaign_id),
            Some("baseline"),
            &baseline_tags,
        );
        let unscored: BTreeSet<String> = baseline_open
            .iter()
            .flat_map(|observation| observation.skipped.iter().cloned())
            .map(|(gauge, reason)| format!("{gauge}: {reason}"))
            .collect();
        if !unscored.is_empty() && !options.json {
            for line in &unscored {
                eprintln!("gauge unscored — {line}");
            }
        }

        let mut proposer: Box<dyn Proposer> = match args.proposer.as_str() {
            "fixture" => Box::new(FixtureProposer::from_env()),
            "native" => Box::new(NativeProposer),
            other => return Err(format!("unknown proposer `{other}` (fixture|native)")),
        };
        let mut prior_failures: Vec<String> = Vec::new();
        let mut cards: Vec<Value> = Vec::new();
        let mut proposed_any = false;
        let mut candidate_seq = 0usize;
        let mut spent_micros: i64 = 0;
        for _round in 0..MAX_PROPOSAL_ROUNDS {
            // The spend cap is a hard ceiling over RECORDED cost. Provider
            // price tables are a follow-on: token-only usage records cost 0,
            // so today the cap binds only where priced costs exist — stated in
            // DR-0037, never silent (the spend events carry the tokens).
            if let Some(cap) = args.spec.spend_cap_micros {
                if spent_micros >= cap {
                    store
                        .append_campaign_event(
                            &campaign_id,
                            "campaign.parked",
                            &json!({"reason": "spend-cap", "spent_micros": spent_micros}),
                        )
                        .map_err(|error| format!("failed to park campaign: {error:?}"))?;
                    break;
                }
            }
            let reflection = build_reflection(
                &source,
                &args.spec,
                &specs,
                &baseline_open,
                &baseline_sealed,
                &open,
                &prior_failures,
                sealing_engaged,
            );
            let Some(proposal) = proposer.propose(&reflection)? else {
                break;
            };
            if proposal.tokens > 0 {
                let cost_micros = 0i64; // unpriced until provider price tables land
                store
                    .append_campaign_event(
                        &campaign_id,
                        "campaign.spend",
                        &json!({"cost_micros": cost_micros, "tokens": proposal.tokens,
                            "what": "proposer turn"}),
                    )
                    .map_err(|error| format!("failed to record spend: {error:?}"))?;
                spent_micros += cost_micros;
            }
            candidate_seq += 1;
            let candidate_id = format!("K-{candidate_seq}");
            let candidate_path = eval_scratch_dir().join(format!("candidate-{candidate_seq}.whip"));
            std::fs::write(&candidate_path, &proposal.source)
                .map_err(|error| format!("failed to stage candidate: {error}"))?;
            let candidate_path_str = candidate_path.to_string_lossy().into_owned();
            // The static gate battery is a free feasibility oracle: candidates
            // that break invariants die before a sample is spent.
            let candidate_ir = match crate::compile_source_path_with_root(
                &candidate_path_str,
                args.root.as_deref(),
            ) {
                Ok((_, candidate_ir)) => candidate_ir,
                Err(error) => {
                    store
                    .append_campaign_event(
                        &campaign_id,
                        "candidate.rejected",
                        &json!({
                            "candidate": candidate_id,
                            "reason": format!("does not compile: {}", compile_failure_summary(&error)),
                            "rationale": proposal.rationale,
                        }),
                    )
                    .map_err(|error| format!("failed to record rejection: {error:?}"))?;
                    prior_failures.push(format!("{candidate_id}: does not compile"));
                    continue;
                }
            };
            let candidate_open = evaluate_all(&candidate_path_str, &candidate_ir, &open, &mut seq)?;
            let candidate_hash = program_hash(&proposal.source);
            record_observations(
                store,
                &candidate_open,
                &specs,
                "regen",
                &candidate_hash,
                Some(&campaign_id),
                Some(&candidate_id),
                &campaign_tags,
            );
            store
                .append_campaign_event(
                    &campaign_id,
                    "candidate.recorded",
                    &json!({
                        "candidate": candidate_id,
                        "hash": candidate_hash,
                        "rationale": proposal.rationale,
                        "source": proposal.source,
                        "baseline_hash": baseline_hash,
                        "proposer": proposer.name(),
                    }),
                )
                .map_err(|error| format!("failed to record candidate: {error:?}"))?;
            let open_verdict =
                dominance_verdict(&specs, &args.spec, &baseline_open, &candidate_open);
            let mut verdict = open_verdict.clone();
            let mut gate_tags = campaign_tags.clone();
            if verdict.proposable && sealing_engaged {
                // Promotion gate: score the sealed holdout on BOTH arms and
                // re-check dominance over the combined evidence. Every gate
                // exposure wears the seal (cumulative, k=3).
                let candidate_sealed =
                    evaluate_all(&candidate_path_str, &candidate_ir, &sealed, &mut seq)?;
                record_observations(
                    store,
                    &candidate_sealed,
                    &specs,
                    "regen",
                    &candidate_hash,
                    Some(&campaign_id),
                    Some(&candidate_id),
                    &campaign_tags,
                );
                for scenario in &sealed {
                    let _ = store.bump_scenario_wear(&scenario.name, WEAR_OUT_AT);
                }
                let combined_base: Vec<RunObservation> = baseline_open
                    .iter()
                    .chain(baseline_sealed.iter())
                    .cloned()
                    .collect();
                let combined_cand: Vec<RunObservation> = candidate_open
                    .iter()
                    .chain(candidate_sealed.iter())
                    .cloned()
                    .collect();
                verdict = dominance_verdict(&specs, &args.spec, &combined_base, &combined_cand);
                if !verdict.proposable {
                    verdict
                        .reasons
                        .push("failed the sealed promotion gate".to_owned());
                    gate_tags.push("holdout-refused".to_owned());
                }
            }
            let card = evidence_card(
                &campaign_id,
                &candidate_id,
                &proposal.rationale,
                &verdict,
                &gate_tags,
                unheld_out,
            );
            if verdict.proposable {
                proposed_any = true;
                store
                    .append_campaign_event(&campaign_id, "candidate.proposed", &card)
                    .map_err(|error| format!("failed to record proposal: {error:?}"))?;
                cards.push(card);
                // Propose-don't-apply: first undominated candidate ends the
                // stage; adoption is the human's move (`whip adopt`).
                break;
            } else if verdict.tradeoff {
                store
                    .append_campaign_event(&campaign_id, "candidate.tradeoff", &card)
                    .map_err(|error| format!("failed to record tradeoff: {error:?}"))?;
                prior_failures.push(format!(
                    "{candidate_id}: genuine tradeoff ({}) — escalated, not accepted",
                    verdict.reasons.join("; ")
                ));
                cards.push(card);
            } else {
                store
                    .append_campaign_event(&campaign_id, "candidate.rejected", &card)
                    .map_err(|error| format!("failed to record rejection: {error:?}"))?;
                prior_failures.push(format!("{candidate_id}: {}", verdict.reasons.join("; ")));
                cards.push(card);
            }
        }
        store
            .append_campaign_event(
                &campaign_id,
                "campaign.closed",
                &json!({"proposed": proposed_any}),
            )
            .map_err(|error| format!("failed to close campaign: {error:?}"))?;
        Ok((cards, proposed_any, candidate_seq))
    })(&mut store);
    let (cards, proposed_any, candidate_seq) = match outcome {
        Ok(parts) => parts,
        Err(message) => {
            // A campaign must never linger "open" after a crash: the record
            // says what happened.
            let _ = store.append_campaign_event(
                &campaign_id,
                "campaign.failed",
                &json!({"reason": message}),
            );
            return Err(message);
        }
    };
    if options.json {
        return Ok(emit_json(json!({
            "schema": "whipplescript.improve.v0",
            "campaign": campaign_id,
            "program": program_path,
            "baseline_hash": baseline_hash,
            "unheld_out": unheld_out,
            "cards": cards,
            "proposed": proposed_any,
        })));
    }
    println!("campaign {campaign_id} on `{program_path}`");
    if unheld_out {
        println!("  tags: unheld-out (fewer than {MIN_SCENARIOS_FOR_SEALING} pinned scenarios)");
    }
    if cards.is_empty() {
        println!("  no candidates produced (proposer exhausted)");
    }
    for card in &cards {
        print_card(card);
    }
    if proposed_any {
        println!("adopt with: whip adopt {campaign_id}:K-{candidate_seq} --program {program_path}");
    }
    Ok(ExitCode::SUCCESS)
}

fn evidence_card(
    campaign_id: &str,
    candidate_id: &str,
    rationale: &str,
    verdict: &CandidateVerdict,
    tags: &[String],
    unheld_out: bool,
) -> Value {
    let mut all_tags = tags.to_vec();
    if unheld_out && !all_tags.iter().any(|tag| tag == "unheld-out") {
        all_tags.push("unheld-out".to_owned());
    }
    json!({
        "candidate": candidate_id,
        "campaign": campaign_id,
        "rationale": rationale,
        "proposable": verdict.proposable,
        "tradeoff": verdict.tradeoff,
        "reasons": verdict.reasons,
        "tags": all_tags,
        "gauges": verdict.lines.iter().map(|line| json!({
            "gauge": line.gauge,
            "role": line.role,
            "delta": match line.delta {
                Delta::Better => "better",
                Delta::InBand => "in-band",
                Delta::Worse => "worse",
                Delta::Unmeasured => "unmeasured",
            },
            "baseline": line.baseline,
            "candidate": line.candidate,
            "band": line.band,
            "bar_met": line.bar_met,
            "reach_met": line.reach_met,
        })).collect::<Vec<_>>(),
    })
}

fn print_card(card: &Value) {
    let candidate = card["candidate"].as_str().unwrap_or("?");
    let status = if card["proposable"].as_bool().unwrap_or(false) {
        "PROPOSED"
    } else if card["tradeoff"].as_bool().unwrap_or(false) {
        "TRADEOFF — your call"
    } else {
        "refused"
    };
    println!("candidate {candidate}: {status}");
    if let Some(rationale) = card["rationale"].as_str() {
        if !rationale.is_empty() {
            println!("  rationale: {rationale}");
        }
    }
    if let Some(gauges) = card["gauges"].as_array() {
        for line in gauges {
            let base = line["baseline"].as_f64().unwrap_or(f64::NAN);
            let cand = line["candidate"].as_f64().unwrap_or(f64::NAN);
            let mut suffix = String::new();
            if let Some(met) = line["bar_met"].as_bool() {
                suffix.push_str(if met { " bar✓" } else { " bar✗" });
            }
            println!(
                "  {} [{}] {:.4} -> {:.4} ({}){}",
                line["gauge"].as_str().unwrap_or("?"),
                line["role"].as_str().unwrap_or("?"),
                base,
                cand,
                line["delta"].as_str().unwrap_or("?"),
                suffix
            );
        }
    }
    if let Some(reasons) = card["reasons"].as_array() {
        for reason in reasons {
            if let Some(reason) = reason.as_str() {
                println!("  · {reason}");
            }
        }
    }
    if let Some(tags) = card["tags"].as_array() {
        if !tags.is_empty() {
            let rendered: Vec<&str> = tags.iter().filter_map(Value::as_str).collect();
            println!("  tags: {}", rendered.join(", "));
        }
    }
}

// ---------------------------------------------------------------------------
// whip campaigns / whip campaign <id> / whip adopt
// ---------------------------------------------------------------------------

pub(crate) fn campaigns_command(options: &CliOptions) -> ExitCode {
    let store = match open_improve_store() {
        Ok(store) => store,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let campaigns = match store.list_campaigns() {
        Ok(campaigns) => campaigns,
        Err(error) => {
            eprintln!("failed to list campaigns: {error:?}");
            return ExitCode::from(2);
        }
    };
    if options.json {
        return emit_json(json!({
            "schema": "whipplescript.campaigns.v0",
            "campaigns": campaigns.iter().map(campaign_summary_json).collect::<Vec<_>>(),
        }));
    }
    if campaigns.is_empty() {
        println!("no campaigns");
        return ExitCode::SUCCESS;
    }
    for campaign in &campaigns {
        println!(
            "{} {} candidates={} proposed={} opened={}",
            campaign.campaign_id,
            campaign.status,
            campaign.candidates,
            campaign.proposed,
            campaign.opened_at
        );
    }
    ExitCode::SUCCESS
}

fn campaign_summary_json(campaign: &CampaignSummary) -> Value {
    json!({
        "campaign": campaign.campaign_id,
        "status": campaign.status,
        "spec": campaign.spec,
        "opened_at": campaign.opened_at,
        "last_event_at": campaign.last_event_at,
        "candidates": campaign.candidates,
        "proposed": campaign.proposed,
        "spent_micros": campaign.spent_micros,
    })
}

pub(crate) fn campaign_detail_command(options: &CliOptions) -> ExitCode {
    let Some(campaign_id) = options.args.first() else {
        eprintln!("usage: whip campaign <id>");
        return ExitCode::from(2);
    };
    let store = match open_improve_store() {
        Ok(store) => store,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let events = match store.list_campaign_events(campaign_id) {
        Ok(events) => events,
        Err(error) => {
            eprintln!("failed to read campaign: {error:?}");
            return ExitCode::from(2);
        }
    };
    if events.is_empty() {
        eprintln!("unknown campaign `{campaign_id}`");
        return ExitCode::from(2);
    }
    if options.json {
        return emit_json(json!({
            "schema": "whipplescript.campaign.v0",
            "campaign": campaign_id,
            "events": events.iter().map(|event| json!({
                "seq": event.seq,
                "type": event.event_type,
                "payload": event.payload,
                "at": event.created_at,
            })).collect::<Vec<_>>(),
        }));
    }
    let mut summary = CampaignSummary {
        campaign_id: campaign_id.clone(),
        status: "open".to_owned(),
        spec: Value::Null,
        opened_at: String::new(),
        last_event_at: String::new(),
        candidates: 0,
        proposed: 0,
        spent_micros: 0,
    };
    for event in &events {
        fold_campaign_event(&mut summary, event);
    }
    println!(
        "campaign {} {} candidates={} proposed={}",
        summary.campaign_id, summary.status, summary.candidates, summary.proposed
    );
    for event in &events {
        match event.event_type.as_str() {
            "candidate.proposed" | "candidate.tradeoff" | "candidate.rejected" => {
                print_card(&event.payload);
            }
            "campaign.sealed" => {
                let sealed = event.payload["sealed"]
                    .as_array()
                    .map(|names| names.len())
                    .unwrap_or(0);
                let open = event.payload["open"]
                    .as_array()
                    .map(|names| names.len())
                    .unwrap_or(0);
                println!("  sealed {sealed} scenario(s), {open} open");
            }
            _ => {}
        }
    }
    ExitCode::SUCCESS
}

pub(crate) fn adopt_command(options: &CliOptions) -> ExitCode {
    match run_adopt(options) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run_adopt(options: &CliOptions) -> Result<ExitCode, String> {
    let mut target = None;
    let mut program = None;
    let mut index = 0;
    while index < options.args.len() {
        match options.args[index].as_str() {
            "--program" => {
                index += 1;
                program = options.args.get(index).cloned();
            }
            other if target.is_none() => target = Some(other.to_owned()),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        index += 1;
    }
    let target = target.ok_or("usage: whip adopt <campaign>:<candidate> --program <path>")?;
    let (campaign_id, candidate_id) = target
        .split_once(':')
        .ok_or("adopt target must be <campaign>:<candidate> (e.g. C-1:K-2)")?;
    let program_path = resolve_program_path(program)?;
    let mut store = open_improve_store()?;
    let events = store
        .list_campaign_events(campaign_id)
        .map_err(|error| format!("failed to read campaign: {error:?}"))?;
    if events.is_empty() {
        return Err(format!("unknown campaign `{campaign_id}`"));
    }
    let recorded = events
        .iter()
        .find(|event| {
            event.event_type == "candidate.recorded"
                && event.payload["candidate"].as_str() == Some(candidate_id)
        })
        .ok_or_else(|| format!("campaign `{campaign_id}` has no candidate `{candidate_id}`"))?;
    // Adoption is only offered for candidates the dominance invariant
    // actually surfaced — a refused or tradeoff candidate is a decision the
    // evidence card escalates, never a silent write path around the model.
    let proposed = events.iter().any(|event| {
        event.event_type == "candidate.proposed"
            && event.payload["candidate"].as_str() == Some(candidate_id)
    });
    if !proposed {
        return Err(format!(
            "candidate `{candidate_id}` was not proposed by campaign `{campaign_id}`              (it was refused or escalated as a tradeoff); adoption is reserved for              proposed candidates"
        ));
    }
    let source = recorded.payload["source"]
        .as_str()
        .ok_or("candidate record carries no source")?;
    let baseline_hash = recorded.payload["baseline_hash"]
        .as_str()
        .ok_or("candidate record carries no baseline hash")?;
    let current = std::fs::read_to_string(&program_path)
        .map_err(|error| format!("failed to read `{program_path}`: {error}"))?;
    // Adoption always merges into CURRENT mainline: if the program moved
    // under the campaign, refuse honestly rather than silently undo a human
    // edit (the certified-merge rebase is the principled upgrade).
    if program_hash(&current) != baseline_hash {
        return Err(format!(
            "`{program_path}` changed since campaign {campaign_id} evaluated its baseline; \
             re-run the campaign against the current program"
        ));
    }
    std::fs::write(&program_path, source)
        .map_err(|error| format!("failed to write `{program_path}`: {error}"))?;
    store
        .append_campaign_event(
            campaign_id,
            "candidate.adopted",
            &json!({
                "candidate": candidate_id,
                "program": program_path,
                "hash": program_hash(source),
            }),
        )
        .map_err(|error| format!("failed to record adoption: {error:?}"))?;
    if options.json {
        return Ok(emit_json(json!({
            "schema": "whipplescript.adopt.v0",
            "campaign": campaign_id,
            "candidate": candidate_id,
            "program": program_path,
        })));
    }
    println!("adopted {campaign_id}:{candidate_id} into `{program_path}`");
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// whip pin / whip gauges / ambient scoring
// ---------------------------------------------------------------------------

pub(crate) fn pin_command(options: &CliOptions) -> ExitCode {
    match run_pin(options) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn run_pin(options: &CliOptions) -> Result<ExitCode, String> {
    let mut instance_id = None;
    let mut name = None;
    let mut index = 0;
    while index < options.args.len() {
        match options.args[index].as_str() {
            "--as" => {
                index += 1;
                name = options.args.get(index).cloned();
            }
            other if instance_id.is_none() => instance_id = Some(other.to_owned()),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        index += 1;
    }
    let instance_id = instance_id.ok_or("usage: whip pin <instance> --as <name>")?;
    let name = name.ok_or("usage: whip pin <instance> --as <name>")?;
    let store = SqliteStore::open(&options.store_path)
        .map_err(|error| format!("failed to open store: {error:?}"))?;
    let instance = store
        .get_instance(&instance_id)
        .map_err(|error| format!("failed to read instance: {error:?}"))?
        .ok_or_else(|| format!("unknown instance `{instance_id}`"))?;
    let mut improve_store = open_improve_store()?;
    improve_store
        .pin_scenario(&name, &instance_id, None, &instance.input_json, None)
        .map_err(|error| format!("failed to pin: {error:?}"))?;
    if options.json {
        return Ok(emit_json(json!({
            "schema": "whipplescript.pin.v0",
            "scenario": name,
            "instance": instance_id,
        })));
    }
    println!("pinned `{name}` from instance {instance_id}");
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn gauges_command(options: &CliOptions) -> ExitCode {
    let store = match open_improve_store() {
        Ok(store) => store,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let filter = options.args.first().map(String::as_str);
    let summaries = match store.evidence_summary(filter) {
        Ok(summaries) => summaries,
        Err(error) => {
            eprintln!("failed to read evidence: {error:?}");
            return ExitCode::from(2);
        }
    };
    if options.json {
        return emit_json(json!({
            "schema": "whipplescript.gauges.v0",
            "gauges": summaries.iter().map(|row| json!({
                "gauge": row.gauge,
                "n": row.n,
                "live": row.live,
                "regen": row.regen,
                "mean": if row.n > 0 { row.score_sum / row.n as f64 } else { 0.0 },
                "passes": row.passes,
            })).collect::<Vec<_>>(),
        }));
    }
    if summaries.is_empty() {
        println!("no gauge evidence yet (runs score ambiently once gauges are declared)");
        return ExitCode::SUCCESS;
    }
    for row in &summaries {
        println!(
            "{}: mean {:.4} · N={} ({} regen · {} live) · passes={}",
            row.gauge,
            row.score_sum / (row.n.max(1)) as f64,
            row.n,
            row.regen,
            row.live,
            row.passes
        );
    }
    ExitCode::SUCCESS
}

/// Ambient scoring hook, called from `whip dev` after a drive loop settles:
/// every run is an observation (research note §4.1). v1 scores the free,
/// deterministic judges (exec + builtins; labels need a scenario); prompt
/// judges are campaign-time. Failures are silent-skip by design here — the
/// ambient stream must never break a dev loop.
pub(crate) fn ambient_score_after_dev(
    store_path: &Path,
    instance_id: &str,
    ir: &IrProgram,
    json: bool,
) {
    if ir.gauges.is_empty() {
        return;
    }
    let Ok(store) = SqliteStore::open(store_path) else {
        return;
    };
    let specs = collect_gauge_specs(ir);
    let observation = score_instance(&store, instance_id, &specs, None, true);
    if observation.readings.is_empty() {
        return;
    }
    let Ok(mut improve_store) = open_improve_store() else {
        return;
    };
    for (gauge, reading) in &observation.readings {
        let scorer = specs
            .iter()
            .find(|spec| &spec.name == gauge)
            .map(|spec| scorer_label(&spec.judge))
            .unwrap_or_else(|| "unknown".to_owned());
        let _ = improve_store.record_evidence(NewEvidence {
            gauge,
            score: reading.score,
            passed: reading.passed,
            instance_id: Some(instance_id),
            program_hash: None,
            branch_ref: None,
            execution_mode: "live",
            scorer: &scorer,
            scenario: None,
            campaign_id: None,
            candidate_id: None,
            cost_micros: 0,
            tags: reading.tags.clone(),
        });
    }
    if !json {
        let rendered: Vec<String> = observation
            .readings
            .iter()
            .map(|(gauge, reading)| format!("{gauge} {:.4}", reading.score))
            .collect();
        println!("gauges scored (ambient): {}", rendered.join(" · "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_quality_no_bar(name: &str) -> GaugeSpec {
        GaugeSpec {
            name: name.to_owned(),
            judge: JudgeSpec::Exec("./judge".to_owned()),
            bar: None,
            inputs: Vec::new(),
            direction_up: true,
            builtin: false,
        }
    }

    fn spec_quality(name: &str) -> GaugeSpec {
        GaugeSpec {
            name: name.to_owned(),
            judge: JudgeSpec::Exec("./judge".to_owned()),
            bar: Some(BarSpec {
                chance_field: Some("ok".to_owned()),
                stat: None,
                ge: true,
                threshold: 0.5,
            }),
            inputs: Vec::new(),
            direction_up: true,
            builtin: false,
        }
    }

    fn observations(gauge: &str, passes: &[bool]) -> Vec<RunObservation> {
        passes
            .iter()
            .enumerate()
            .map(|(index, passed)| RunObservation {
                scenario: Some(format!("s{index}")),
                readings: BTreeMap::from([(
                    gauge.to_owned(),
                    GaugeReading {
                        score: if *passed { 1.0 } else { 0.0 },
                        passed: Some(*passed),
                        tags: Vec::new(),
                    },
                )]),
                skipped: Vec::new(),
            })
            .collect()
    }

    fn merge(a: Vec<RunObservation>, b: Vec<RunObservation>) -> Vec<RunObservation> {
        a.into_iter()
            .zip(b)
            .map(|(mut left, right)| {
                left.readings.extend(right.readings);
                left
            })
            .collect()
    }

    #[test]
    fn parse_targets_and_stages() {
        let args: Vec<String> = [
            "extract_quality>=0.9",
            "then",
            "std.spend",
            "--sacrifice",
            "verbosity",
            "--within",
            "tone=2%",
            "--spend-cap",
            "$4",
            "--proposer",
            "fixture",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let parsed = parse_improve_args(&args, &[]).expect("parses");
        assert_eq!(parsed.spec.ascend.len(), 1);
        assert_eq!(parsed.spec.ascend[0].0, "extract_quality");
        let reach = parsed.spec.ascend[0].1.as_ref().expect("reach target");
        assert!(reach.ge);
        assert!((reach.threshold - 0.9).abs() < 1e-9);
        assert_eq!(parsed.spec.later_stages.len(), 1);
        assert_eq!(parsed.spec.later_stages[0][0], "std.spend");
        assert_eq!(parsed.spec.sacrifice, vec!["verbosity"]);
        assert_eq!(parsed.spec.within_percent.get("tone"), Some(&2.0));
        assert_eq!(parsed.spec.spend_cap_micros, Some(4_000_000));
        assert!(!parsed.spec.repair);
    }

    #[test]
    fn bare_improve_is_repair_mode() {
        let parsed = parse_improve_args(&[], &[]).expect("parses");
        assert!(parsed.spec.repair);
    }

    #[test]
    fn dominance_accepts_undominated_candidate() {
        let specs = vec![spec_quality("focus"), spec_quality("guarded")];
        let campaign = CampaignSpec {
            ascend: vec![("focus".to_owned(), None)],
            ..Default::default()
        };
        let base = merge(
            observations("focus", &[false, false, false, true]),
            observations("guarded", &[true, true, true, true]),
        );
        let cand = merge(
            observations("focus", &[true, true, true, true]),
            observations("guarded", &[true, true, true, true]),
        );
        let verdict = dominance_verdict(&specs, &campaign, &base, &cand);
        assert!(verdict.proposable, "reasons: {:?}", verdict.reasons);
        assert!(!verdict.tradeoff);
    }

    #[test]
    fn dominance_refuses_guard_regression() {
        let specs = vec![spec_quality("focus"), spec_quality_no_bar("guarded")];
        let campaign = CampaignSpec {
            ascend: vec![("focus".to_owned(), None)],
            ..Default::default()
        };
        let base = merge(
            observations("focus", &[false, false, false, false]),
            observations("guarded", &[true, true, true, true]),
        );
        let cand = merge(
            observations("focus", &[true, true, true, true]),
            observations("guarded", &[false, false, false, false]),
        );
        let verdict = dominance_verdict(&specs, &campaign, &base, &cand);
        assert!(!verdict.proposable);
        assert!(verdict.tradeoff, "focus up + guard broken is a tradeoff");
        assert!(verdict
            .reasons
            .iter()
            .any(|reason| reason.contains("guarded")));
    }

    #[test]
    fn dominance_sacrifice_releases_guard() {
        let specs = vec![spec_quality("focus"), spec_quality_no_bar("guarded")];
        let campaign = CampaignSpec {
            ascend: vec![("focus".to_owned(), None)],
            sacrifice: vec!["guarded".to_owned()],
            ..Default::default()
        };
        let base = merge(
            observations("focus", &[false, false, false, false]),
            observations("guarded", &[true, true, true, true]),
        );
        let cand = merge(
            observations("focus", &[true, true, true, true]),
            observations("guarded", &[false, false, false, false]),
        );
        let verdict = dominance_verdict(&specs, &campaign, &base, &cand);
        assert!(
            verdict.proposable,
            "sacrificed regression is released: {:?}",
            verdict.reasons
        );
    }

    #[test]
    fn dominance_refuses_bar_violation() {
        let specs = vec![spec_quality("focus")];
        let campaign = CampaignSpec {
            ascend: vec![("focus".to_owned(), None)],
            ..Default::default()
        };
        // Improves from 0/4 to 1/4 — better than baseline but the declared
        // bar (>= 0.5) is still violated.
        let base = observations("focus", &[false, false, false, false]);
        let cand = observations("focus", &[true, false, false, false]);
        let verdict = dominance_verdict(&specs, &campaign, &base, &cand);
        assert!(!verdict.proposable, "bars are hard constraints");
    }

    #[test]
    fn resource_gauge_uses_relative_band_and_descends() {
        let spec = GaugeSpec {
            name: "std.latency".to_owned(),
            judge: JudgeSpec::Builtin,
            bar: None,
            inputs: Vec::new(),
            direction_up: false,
            builtin: true,
        };
        let base = GaugeAggregate {
            scores: vec![1000.0, 1000.0],
            passes: Vec::new(),
        };
        let better = GaugeAggregate {
            scores: vec![800.0, 800.0],
            passes: Vec::new(),
        };
        let noise = GaugeAggregate {
            scores: vec![1020.0, 1020.0],
            passes: Vec::new(),
        };
        let (verdict, _, _) = delta_verdict(&spec, &base, &better, None);
        assert_eq!(verdict, Delta::Better, "lower latency is better");
        let (verdict, _, _) = delta_verdict(&spec, &base, &noise, None);
        assert_eq!(verdict, Delta::InBand, "+2% sits inside the 5% band");
    }

    #[test]
    fn sealing_respects_floor_and_degeneracy() {
        let scenario = |name: &str| ScenarioRow {
            name: name.to_owned(),
            instance_id: "i".to_owned(),
            workflow: None,
            input_json: "{}".to_owned(),
            program_hash: None,
            pinned_at: String::new(),
            retired: false,
            wear: 0,
        };
        let few = vec![scenario("a"), scenario("b"), scenario("c")];
        let (open, sealed, engaged) = seal_scenarios("C-1", &few);
        assert!(!engaged, "below the floor the campaign runs unheld-out");
        assert_eq!(open.len(), 3);
        assert!(sealed.is_empty());
        let many: Vec<ScenarioRow> = (0..10)
            .map(|index| scenario(&format!("s{index}")))
            .collect();
        let (open, sealed, engaged) = seal_scenarios("C-1", &many);
        assert!(engaged);
        assert_eq!(sealed.len(), 2, "20% of 10, floor 2");
        assert_eq!(open.len(), 8);
        // Deterministic per campaign id.
        let (_, sealed_again, _) = seal_scenarios("C-1", &many);
        assert_eq!(
            sealed.iter().map(|s| &s.name).collect::<Vec<_>>(),
            sealed_again.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        // A different campaign seals a (generally) different subset — rotation.
        let (_, sealed_other, _) = seal_scenarios("C-2", &many);
        assert_ne!(
            sealed.iter().map(|s| &s.name).collect::<Vec<_>>(),
            sealed_other.iter().map(|s| &s.name).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn reflection_is_holdout_blind() {
        let campaign = CampaignSpec {
            ascend: vec![("focus".to_owned(), None)],
            ..Default::default()
        };
        let specs = vec![spec_quality("focus")];
        let open_observations = observations("focus", &[false]);
        let sealed_observations = vec![RunObservation {
            scenario: Some("sealed-secret".to_owned()),
            readings: BTreeMap::from([(
                "focus".to_owned(),
                GaugeReading {
                    score: 0.0,
                    passed: Some(false),
                    tags: Vec::new(),
                },
            )]),
            skipped: Vec::new(),
        }];
        let reflection = build_reflection(
            "workflow X",
            &campaign,
            &specs,
            &open_observations,
            &sealed_observations,
            &[],
            &[],
            true,
        );
        assert!(
            !reflection.contains("sealed-secret"),
            "sealed scenario names/traces must never reach the proposer"
        );
        assert!(
            reflection.contains("Sealed holdout"),
            "aggregates over sealed scenarios are allowed"
        );
    }

    #[test]
    fn repair_mode_refuses_a_no_op_and_proposes_a_restore() {
        let specs = vec![spec_quality("focus")];
        let campaign = CampaignSpec {
            repair: true,
            ..Default::default()
        };
        // Healthy baseline, healthy candidate: nothing to repair, refuse.
        let healthy = observations("focus", &[true, true, true, true]);
        let verdict = dominance_verdict(&specs, &campaign, &healthy, &healthy);
        assert!(
            !verdict.proposable,
            "a no-op on a healthy program repairs nothing"
        );
        // Violated baseline restored by the candidate: proposable.
        let broken = observations("focus", &[false, false, false, false]);
        let restored = observations("focus", &[true, true, true, true]);
        let verdict = dominance_verdict(&specs, &campaign, &broken, &restored);
        assert!(verdict.proposable, "reasons: {:?}", verdict.reasons);
    }

    #[test]
    fn reach_target_ratchets_once_achieved() {
        let specs = vec![spec_quality("focus"), spec_quality_no_bar("helper")];
        let campaign = CampaignSpec {
            ascend: vec![
                (
                    "focus".to_owned(),
                    Some(ReachTarget {
                        ge: true,
                        threshold: 0.75,
                        raw: "0.75".to_owned(),
                    }),
                ),
                ("helper".to_owned(), None),
            ],
            ..Default::default()
        };
        // Baseline met the target (1.0 >= 0.75); the candidate improves the
        // other ascend gauge but drops focus back below the target — the
        // achieved level is a hard bound, refuse.
        let base = merge(
            observations("focus", &[true, true, true, true]),
            observations("helper", &[false, false, false, false]),
        );
        let cand = merge(
            observations("focus", &[true, true, false, false]),
            observations("helper", &[true, true, true, true]),
        );
        let verdict = dominance_verdict(&specs, &campaign, &base, &cand);
        assert!(!verdict.proposable, "ratchet must hold the achieved reach");
        assert!(verdict
            .reasons
            .iter()
            .any(|reason| reason.contains("ratchet")));
    }

    #[test]
    fn bar_stat_quantiles() {
        let aggregate = GaugeAggregate {
            scores: (1..=100).map(|value| value as f64).collect(),
            passes: Vec::new(),
        };
        let bar = BarSpec {
            chance_field: None,
            stat: Some("p90".to_owned()),
            ge: false,
            threshold: 95.0,
        };
        let value = bar_stat(&aggregate, &bar).expect("p90 computes");
        assert!((value - 90.0).abs() <= 1.0);
        assert_eq!(bar_met(&aggregate, &bar), Some(true));
    }
}
