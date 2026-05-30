use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AdmissionPolicy, WhippleScriptError, WhippleScriptResult, SupervisionPolicy};

pub const CONFIG_DIR_NAME: &str = ".whipplescript";
pub const CONFIG_FILE_NAME: &str = "project.whip";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
    config_path: PathBuf,
}

impl Workspace {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.join(CONFIG_DIR_NAME)
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhippleScriptConfig {
    pub version: String,
    pub tasks: Vec<TaskConfig>,
    pub services: Vec<ServiceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskConfig {
    pub name: String,
    pub run: String,
    pub trigger: TriggerConfig,
    pub admission: AdmissionConfig,
    pub supervision: SupervisionPolicyConfig,
    pub resources: ResourcePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceConfig {
    pub name: String,
    pub run: String,
    pub enabled: bool,
    pub supervision: SupervisionPolicyConfig,
    pub health: Option<HealthCheckConfig>,
    pub resources: ResourcePolicy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TriggerConfig {
    pub schedule: Option<String>,
    pub watch: Vec<String>,
    pub on: Option<String>,
    pub settle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdmissionConfig {
    pub when_busy: AdmissionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupervisionPolicyConfig {
    pub restart: RestartMode,
    pub max_restarts: Option<u32>,
    pub within: Option<String>,
    pub backoff: Option<BackoffMode>,
    pub start_delay: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthCheckConfig {
    pub check: String,
    pub every: String,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ResourcePolicy {
    pub kill_after: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartMode {
    Never,
    OnFailure,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffMode {
    Fixed,
    Exponential,
}

impl Default for AdmissionConfig {
    fn default() -> Self {
        Self {
            when_busy: AdmissionPolicy::Allow,
        }
    }
}

impl Default for SupervisionPolicyConfig {
    fn default() -> Self {
        Self {
            restart: RestartMode::Never,
            max_restarts: None,
            within: None,
            backoff: None,
            start_delay: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WhippleScriptConfigFile {
    #[serde(default, rename = "task")]
    tasks: Vec<TaskConfigFile>,
    #[serde(default, rename = "service")]
    services: Vec<ServiceConfigFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskConfigFile {
    name: String,
    run: String,
    schedule: Option<String>,
    #[serde(default)]
    watch: Vec<String>,
    on: Option<String>,
    settle: Option<String>,
    admission: Option<AdmissionConfigFile>,
    supervision: Option<SupervisionPolicyFile>,
    resources: Option<ResourcePolicyFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceConfigFile {
    name: String,
    run: String,
    enabled: Option<bool>,
    supervision: Option<SupervisionPolicyFile>,
    health: Option<HealthCheckFile>,
    resources: Option<ResourcePolicyFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionConfigFile {
    when_busy: AdmissionPolicy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupervisionPolicyFile {
    restart: Option<RestartMode>,
    max_restarts: Option<u32>,
    within: Option<String>,
    backoff: Option<BackoffMode>,
    start_delay: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthCheckFile {
    check: String,
    every: String,
    timeout: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourcePolicyFile {
    kill_after: Option<String>,
}

pub fn discover_workspace(start: impl AsRef<Path>) -> WhippleScriptResult<Workspace> {
    discover_workspace_from(start.as_ref())
}

pub fn resolve_workspace(
    explicit_workspace: Option<impl AsRef<Path>>,
    start: impl AsRef<Path>,
) -> WhippleScriptResult<Workspace> {
    if let Some(path) = explicit_workspace {
        return workspace_from_explicit(path.as_ref());
    }

    discover_workspace(start)
}

pub fn load_config(path: impl AsRef<Path>) -> WhippleScriptResult<WhippleScriptConfig> {
    let config_path = path.as_ref();
    let raw = fs::read_to_string(config_path).map_err(|error| {
        WhippleScriptError::invalid_input(format!(
            "failed to read config {}: {}",
            config_path.display(),
            error
        ))
    })?;
    parse_config(&raw)
}

pub fn load_workspace_config(workspace: &Workspace) -> WhippleScriptResult<WhippleScriptConfig> {
    load_config(workspace.config_path())
}

fn discover_workspace_from(start: &Path) -> WhippleScriptResult<Workspace> {
    let canonical_start = fs::canonicalize(start).map_err(|error| {
        WhippleScriptError::invalid_input(format!(
            "failed to resolve workspace start path {}: {}",
            start.display(),
            error
        ))
    })?;
    let start_dir = if canonical_start.is_dir() {
        canonical_start
    } else {
        canonical_start
            .parent()
            .ok_or_else(|| {
                WhippleScriptError::invalid_input("workspace discovery requires a directory")
            })?
            .to_path_buf()
    };

    for ancestor in start_dir.ancestors() {
        let config_path = ancestor.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME);
        if config_path.is_file() {
            return Ok(Workspace {
                root: ancestor.to_path_buf(),
                config_path,
            });
        }
    }

    Err(WhippleScriptError::not_found(format!(
        "no {} found while searching upward from {}",
        workspace_config_display(),
        start.display()
    )))
}

fn workspace_from_explicit(path: &Path) -> WhippleScriptResult<Workspace> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        WhippleScriptError::invalid_input(format!(
            "failed to resolve explicit workspace path {}: {}",
            path.display(),
            error
        ))
    })?;

    let config_path = if canonical.is_file() {
        canonical
    } else {
        canonical.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME)
    };

    if !config_path.is_file() {
        return Err(WhippleScriptError::not_found(format!(
            "explicit workspace does not contain {}: {}",
            workspace_config_display(),
            config_path.display()
        )));
    }

    let root = config_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| WhippleScriptError::invalid_input("invalid workspace config path"))?
        .to_path_buf();

    Ok(Workspace { root, config_path })
}

fn parse_config(raw: &str) -> WhippleScriptResult<WhippleScriptConfig> {
    let value: toml::Value = toml::from_str(raw)
        .map_err(|error| WhippleScriptError::invalid_input(format!("invalid TOML config: {error}")))?;
    reject_forbidden_blocks(&value)?;

    let parsed: WhippleScriptConfigFile = value.try_into().map_err(|error| {
        WhippleScriptError::invalid_input(format!("invalid WhippleScript config shape: {error}"))
    })?;

    normalize_config(parsed)
}

fn normalize_config(config: WhippleScriptConfigFile) -> WhippleScriptResult<WhippleScriptConfig> {
    let mut task_names = HashSet::new();
    let mut tasks = Vec::with_capacity(config.tasks.len());
    for task in config.tasks {
        let normalized = normalize_task(task)?;
        if !task_names.insert(normalized.name.clone()) {
            return Err(WhippleScriptError::invalid_input(format!(
                "duplicate task name: {}",
                normalized.name
            )));
        }
        tasks.push(normalized);
    }

    let mut service_names = HashSet::new();
    let mut services = Vec::with_capacity(config.services.len());
    for service in config.services {
        let normalized = normalize_service(service)?;
        if !service_names.insert(normalized.name.clone()) {
            return Err(WhippleScriptError::invalid_input(format!(
                "duplicate service name: {}",
                normalized.name
            )));
        }
        services.push(normalized);
    }

    let mut normalized = WhippleScriptConfig {
        version: String::new(),
        tasks,
        services,
    };
    normalized.version = config_version(&normalized)?;
    Ok(normalized)
}

fn normalize_task(task: TaskConfigFile) -> WhippleScriptResult<TaskConfig> {
    let name = validate_name("task", task.name)?;
    let run = validate_command("task", &name, task.run)?;
    let trigger = normalize_trigger(&name, task.schedule, task.watch, task.on, task.settle)?;
    let admission = AdmissionConfig {
        when_busy: task
            .admission
            .map(|admission| admission.when_busy)
            .unwrap_or(AdmissionPolicy::Allow),
    };
    let supervision = normalize_supervision("task", &name, task.supervision)?;
    let resources = normalize_resources("task", &name, task.resources)?;

    Ok(TaskConfig {
        name,
        run,
        trigger,
        admission,
        supervision,
        resources,
    })
}

fn normalize_service(service: ServiceConfigFile) -> WhippleScriptResult<ServiceConfig> {
    let name = validate_name("service", service.name)?;
    let run = validate_command("service", &name, service.run)?;
    let supervision = normalize_supervision("service", &name, service.supervision)?;
    let health = service
        .health
        .map(|health| normalize_health(&name, health))
        .transpose()?;
    let resources = normalize_resources("service", &name, service.resources)?;

    Ok(ServiceConfig {
        name,
        run,
        enabled: service.enabled.unwrap_or(true),
        supervision,
        health,
        resources,
    })
}

fn normalize_trigger(
    task_name: &str,
    schedule: Option<String>,
    watch: Vec<String>,
    on: Option<String>,
    settle: Option<String>,
) -> WhippleScriptResult<TriggerConfig> {
    let schedule = schedule
        .map(|value| {
            validate_nonempty(format!("task {task_name} schedule").as_str(), value)
                .and_then(|valid| validate_cron(task_name, valid))
        })
        .transpose()?;
    let watch = normalize_watch_patterns(task_name, watch)?;
    let on = on
        .map(|value| {
            validate_nonempty(format!("task {task_name} event trigger").as_str(), value)
                .and_then(|valid| validate_event_type(task_name, valid))
        })
        .transpose()?;
    let settle = settle
        .map(|value| {
            validate_nonempty(format!("task {task_name} settle").as_str(), value)
                .and_then(|valid| validate_duration_literal("task settle", task_name, valid))
        })
        .transpose()?;

    if settle.is_some() && watch.is_empty() {
        return Err(WhippleScriptError::invalid_input(format!(
            "task {task_name} sets settle without any watch patterns"
        )));
    }

    Ok(TriggerConfig {
        schedule,
        watch,
        on,
        settle,
    })
}

fn normalize_supervision(
    kind: &str,
    name: &str,
    supervision: Option<SupervisionPolicyFile>,
) -> WhippleScriptResult<SupervisionPolicyConfig> {
    let Some(supervision) = supervision else {
        return Ok(SupervisionPolicyConfig::default());
    };

    let restart = supervision.restart.unwrap_or(RestartMode::Never);
    let within = supervision
        .within
        .map(|value| validate_duration_literal("supervision.within", name, value))
        .transpose()?;
    let start_delay = supervision
        .start_delay
        .map(|value| validate_duration_literal("supervision.start_delay", name, value))
        .transpose()?;

    if (supervision.max_restarts.is_some() || within.is_some() || supervision.backoff.is_some())
        && restart == RestartMode::Never
    {
        return Err(WhippleScriptError::invalid_input(format!(
            "{kind} {name} sets crash-loop controls but supervision.restart is never"
        )));
    }

    if supervision.max_restarts.is_some() ^ within.is_some() {
        return Err(WhippleScriptError::invalid_input(format!(
            "{kind} {name} must set both supervision.max_restarts and supervision.within together"
        )));
    }

    let config = SupervisionPolicyConfig {
        restart,
        max_restarts: supervision.max_restarts,
        within,
        backoff: supervision.backoff,
        start_delay,
    };

    if kind == "task" && config != SupervisionPolicyConfig::default() {
        return Err(WhippleScriptError::invalid_input(format!(
            "task {name} sets non-default supervision, but task restart supervision is not supported; remove [task.supervision] or set restart = \"never\" with no restart controls"
        )));
    }

    Ok(config)
}

fn normalize_health(
    service_name: &str,
    health: HealthCheckFile,
) -> WhippleScriptResult<HealthCheckConfig> {
    let check = validate_command("health check", service_name, health.check)?;
    let every = validate_duration_literal("health.every", service_name, health.every)?;
    let timeout = health
        .timeout
        .map(|value| validate_duration_literal("health.timeout", service_name, value))
        .transpose()?;

    Ok(HealthCheckConfig {
        check,
        every,
        timeout,
    })
}

fn normalize_resources(
    kind: &str,
    name: &str,
    resources: Option<ResourcePolicyFile>,
) -> WhippleScriptResult<ResourcePolicy> {
    let Some(resources) = resources else {
        return Ok(ResourcePolicy::default());
    };

    let kill_after = resources
        .kill_after
        .map(|value| validate_duration_literal("resources.kill_after", name, value))
        .transpose()?;

    if kill_after.is_none() {
        return Err(WhippleScriptError::invalid_input(format!(
            "{kind} {name} has an empty resources block"
        )));
    }

    Ok(ResourcePolicy { kill_after })
}

fn config_version(config: &WhippleScriptConfig) -> WhippleScriptResult<String> {
    let bytes = serde_json::to_vec(&SerializableConfigVersion {
        tasks: &config.tasks,
        services: &config.services,
    })
    .map_err(|error| {
        WhippleScriptError::internal(format!("failed to encode config version: {error}"))
    })?;
    let digest = Sha256::digest(bytes);
    Ok(format!("cfg_{:x}", digest))
}

#[derive(Serialize)]
struct SerializableConfigVersion<'a> {
    tasks: &'a [TaskConfig],
    services: &'a [ServiceConfig],
}

fn validate_name(kind: &str, value: String) -> WhippleScriptResult<String> {
    let value = validate_nonempty(format!("{kind} name").as_str(), value)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
    {
        Ok(value)
    } else {
        Err(WhippleScriptError::invalid_input(format!(
            "{kind} name {value:?} must use only ASCII letters, numbers, '.', '_', '-', or ':'"
        )))
    }
}

fn validate_command(kind: &str, name: &str, value: String) -> WhippleScriptResult<String> {
    validate_nonempty(format!("{kind} {name} run").as_str(), value)
}

fn validate_nonempty(context: &str, value: String) -> WhippleScriptResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(WhippleScriptError::invalid_input(format!(
            "{context} must not be empty"
        )))
    } else {
        Ok(trimmed.to_string())
    }
}

fn validate_cron(task_name: &str, value: String) -> WhippleScriptResult<String> {
    let fields = value.split_whitespace().count();
    if matches!(fields, 5 | 6) {
        Ok(value)
    } else {
        Err(WhippleScriptError::invalid_input(format!(
            "task {task_name} schedule must use 5 or 6 cron fields"
        )))
    }
}

fn normalize_watch_patterns(task_name: &str, patterns: Vec<String>) -> WhippleScriptResult<Vec<String>> {
    let mut normalized = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let pattern =
            validate_nonempty(format!("task {task_name} watch pattern").as_str(), pattern)?;
        normalized.push(pattern);
    }
    Ok(normalized)
}

fn validate_event_type(task_name: &str, value: String) -> WhippleScriptResult<String> {
    if value.chars().any(char::is_whitespace) {
        Err(WhippleScriptError::invalid_input(format!(
            "task {task_name} event trigger must not contain whitespace"
        )))
    } else {
        Ok(value)
    }
}

fn validate_duration_literal(field: &str, name: &str, value: String) -> WhippleScriptResult<String> {
    let value = validate_nonempty(format!("{field} for {name}").as_str(), value)?;
    let split_at = value.find(|ch: char| !ch.is_ascii_digit()).ok_or_else(|| {
        WhippleScriptError::invalid_input(format!("{field} for {name} must include a unit"))
    })?;
    let (digits, unit) = value.split_at(split_at);
    if digits.starts_with('0') && digits.len() > 1 {
        return Err(WhippleScriptError::invalid_input(format!(
            "{field} for {name} must not use zero-padded values"
        )));
    }
    if digits.parse::<u64>().ok().filter(|n| *n > 0).is_none() {
        return Err(WhippleScriptError::invalid_input(format!(
            "{field} for {name} must use a positive integer duration"
        )));
    }
    if matches!(unit, "ms" | "s" | "m" | "h" | "d") {
        Ok(value)
    } else {
        Err(WhippleScriptError::invalid_input(format!(
            "{field} for {name} must end with one of: ms, s, m, h, d"
        )))
    }
}

fn reject_forbidden_blocks(value: &toml::Value) -> WhippleScriptResult<()> {
    let forbidden = [
        (
            "recipe",
            "recipes are scaffolding, not privileged runtime config",
        ),
        (
            "recipes",
            "recipes are scaffolding, not privileged runtime config",
        ),
        (
            "workflow",
            "workflow semantics are outside the v0.3 config boundary",
        ),
        ("plan", "whip plan is outside the v0.3 config boundary"),
        ("dag", "workflow DAGs are outside the v0.3 config boundary"),
        (
            "agent_graph",
            "agent graphs are outside the v0.3 config boundary",
        ),
        (
            "capabilities",
            "capabilities are out of scope for the core v0.3 config model",
        ),
    ];
    reject_forbidden_blocks_at_path(value, "", &forbidden)
}

fn reject_forbidden_blocks_at_path(
    value: &toml::Value,
    path: &str,
    forbidden: &[(&str, &str)],
) -> WhippleScriptResult<()> {
    match value {
        toml::Value::Table(table) => {
            for (key, child) in table {
                if let Some((_, message)) = forbidden
                    .iter()
                    .find(|(forbidden_key, _)| key == forbidden_key)
                {
                    let location = if path.is_empty() {
                        key.to_string()
                    } else {
                        format!("{path}.{key}")
                    };
                    return Err(WhippleScriptError::invalid_input(format!(
                        "unsupported config block {location}: {message}"
                    )));
                }
                let child_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                reject_forbidden_blocks_at_path(child, &child_path, forbidden)?;
            }
            Ok(())
        }
        toml::Value::Array(values) => {
            for value in values {
                reject_forbidden_blocks_at_path(value, path, forbidden)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn workspace_config_display() -> &'static str {
    ".whipplescript/project.whip"
}

impl From<RestartMode> for SupervisionPolicy {
    fn from(value: RestartMode) -> Self {
        SupervisionPolicy {
            restart: Some(
                match value {
                    RestartMode::Never => "never",
                    RestartMode::OnFailure => "on_failure",
                    RestartMode::Always => "always",
                }
                .to_string(),
            ),
            max_restarts: None,
            within: None,
            backoff: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{discover_workspace, load_config, resolve_workspace, AdmissionPolicy, RestartMode};

    #[test]
    fn discovers_nearest_ancestor_workspace_only() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        let nested = root.join("apps/service/src");
        fs::create_dir_all(root.join(".whipplescript")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            root.join(".whipplescript/project.whip"),
            "[[task]]\nname = \"hello\"\nrun = \"echo hi\"\n",
        )
        .unwrap();

        let workspace = discover_workspace(&nested).unwrap();
        assert_eq!(workspace.root(), root.as_path());
        assert_eq!(
            workspace.config_path(),
            root.join(".whipplescript/project.whip")
        );
    }

    #[test]
    fn does_not_search_downward_for_workspace() {
        let temp = tempdir().unwrap();
        let parent = temp.path().join("parent");
        let child = parent.join("child");
        fs::create_dir_all(child.join(".whipplescript")).unwrap();
        fs::write(
            child.join(".whipplescript/project.whip"),
            "[[task]]\nname = \"hello\"\nrun = \"echo hi\"\n",
        )
        .unwrap();

        let error = discover_workspace(&parent).unwrap_err();
        assert!(error
            .to_string()
            .contains("no .whipplescript/project.whip found while searching upward"));
    }

    #[test]
    fn explicit_workspace_override_loads_root() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir_all(root.join(".whipplescript")).unwrap();
        fs::write(
            root.join(".whipplescript/project.whip"),
            "[[task]]\nname = \"hello\"\nrun = \"echo hi\"\n",
        )
        .unwrap();

        let workspace = resolve_workspace(Some(&root), temp.path()).unwrap();
        assert_eq!(workspace.root(), root.as_path());
    }

    #[test]
    fn computes_stable_normalized_versions() {
        let temp = tempdir().unwrap();
        let one = temp.path().join("one.toml");
        let two = temp.path().join("two.toml");
        fs::write(
            &one,
            "[[task]]\nname = \"test\"\nwatch = [\"src/**/*.ts\"]\nrun = \"npm test\"\n",
        )
        .unwrap();
        fs::write(
            &two,
            "[[task]]\nname = \"test\"\nwatch = [\"src/**/*.ts\"]\nrun = \"npm test\"\n\n[task.admission]\nwhen_busy = \"allow\"\n",
        )
        .unwrap();

        let left = load_config(&one).unwrap();
        let right = load_config(&two).unwrap();
        assert_eq!(left.version, right.version);
        assert_eq!(left.tasks[0].admission.when_busy, AdmissionPolicy::Allow);
    }

    #[test]
    fn rejects_invalid_trigger_settle_without_watch() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("project.whip");
        fs::write(
            &path,
            "[[task]]\nname = \"test\"\nsettle = \"300ms\"\nrun = \"npm test\"\n",
        )
        .unwrap();

        let error = load_config(&path).unwrap_err();
        assert!(error
            .to_string()
            .contains("sets settle without any watch patterns"));
    }

    #[test]
    fn rejects_forbidden_recipe_blocks() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("project.whip");
        fs::write(&path, "[recipe]\nname = \"external-review-loop\"\n").unwrap();

        let error = load_config(&path).unwrap_err();
        assert!(error.to_string().contains("recipes are scaffolding"));
    }

    #[test]
    fn rejects_partial_crash_loop_policy() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("project.whip");
        fs::write(
            &path,
            "[[service]]\nname = \"source\"\nrun = \"tsx sources/tool.ts\"\n\n[service.supervision]\nrestart = \"on_failure\"\nmax_restarts = 5\n",
        )
        .unwrap();

        let error = load_config(&path).unwrap_err();
        assert!(error
            .to_string()
            .contains("must set both supervision.max_restarts and supervision.within together"));
    }

    #[test]
    fn rejects_non_default_task_supervision() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("project.whip");
        fs::write(
            &path,
            "[[task]]\nname = \"once\"\nrun = \"false\"\n\n[task.supervision]\nrestart = \"on_failure\"\n",
        )
        .unwrap();

        let error = load_config(&path).unwrap_err();
        assert!(error
            .to_string()
            .contains("task once sets non-default supervision"));
    }

    #[test]
    fn parses_services_with_health_and_resources() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("project.whip");
        fs::write(
            &path,
            "[[service]]\nname = \"source\"\nrun = \"tsx sources/tool.ts\"\n\n[service.health]\ncheck = \"tsx sources/health.ts\"\nevery = \"30s\"\ntimeout = \"5s\"\n\n[service.supervision]\nrestart = \"always\"\nstart_delay = \"1s\"\n\n[service.resources]\nkill_after = \"30m\"\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.services[0].supervision.restart, RestartMode::Always);
        assert_eq!(
            config.services[0].health.as_ref().unwrap().every,
            "30s".to_string()
        );
        assert_eq!(
            config.services[0].resources.kill_after.as_deref(),
            Some("30m")
        );
    }
}
