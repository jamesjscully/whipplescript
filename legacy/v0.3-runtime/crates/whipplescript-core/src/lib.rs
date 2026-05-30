pub mod config;
pub mod error;
pub mod ids;
pub mod model;
pub mod state;

pub use config::{
    discover_workspace, load_config, load_workspace_config, resolve_workspace, AdmissionConfig,
    WhippleScriptConfig, BackoffMode, HealthCheckConfig, ResourcePolicy, RestartMode, ServiceConfig,
    SupervisionPolicyConfig, TaskConfig, TriggerConfig, Workspace, CONFIG_DIR_NAME,
    CONFIG_FILE_NAME,
};
pub use error::{WhippleScriptError, WhippleScriptResult, ErrorKind};
pub use ids::{EventId, RunId, TriggerId, WorkspaceId};
pub use model::{
    AdmissionPolicy, EventRecord, EventRouting, LogRecord, ProcessState, RunOrigin, RunRecord,
    RuntimeSnapshot, ServiceDefinition, SupervisionPolicy, TaskDefinition, TriggerDefinition,
    TriggerOutcome, TriggerRecord,
};
pub use state::{
    state_home_from_env, RunPaths, WorkspaceRuntimePaths, WHIPPLESCRIPT_STATE_DIR_NAME,
    DATABASE_FILE_NAME, DEFAULT_STATE_HOME_SUFFIX, PID_FILE_NAME, RUNS_DIR_NAME, SOCKET_FILE_NAME,
    WORKSPACES_STATE_DIR_NAME, WORKSPACE_LOCK_FILE_NAME,
};
