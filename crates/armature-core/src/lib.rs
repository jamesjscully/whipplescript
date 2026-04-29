pub mod config;
pub mod error;
pub mod ids;
pub mod model;
pub mod state;

pub use config::{
    discover_workspace, load_config, load_workspace_config, resolve_workspace, AdmissionConfig,
    ArmatureConfig, BackoffMode, HealthCheckConfig, ResourcePolicy, RestartMode, ServiceConfig,
    SupervisionPolicyConfig, TaskConfig, TriggerConfig, Workspace, CONFIG_DIR_NAME,
    CONFIG_FILE_NAME,
};
pub use error::{ArmatureError, ArmatureResult, ErrorKind};
pub use ids::{EventId, RunId, WorkspaceId};
pub use model::{
    AdmissionPolicy, EventRecord, EventRouting, LogRecord, ProcessState, RunOrigin, RunRecord,
    RuntimeSnapshot, ServiceDefinition, SupervisionPolicy, TaskDefinition, TriggerDefinition,
};
pub use state::{
    state_home_from_env, RunPaths, WorkspaceRuntimePaths, ARMATURE_STATE_DIR_NAME,
    DATABASE_FILE_NAME, DEFAULT_STATE_HOME_SUFFIX, PID_FILE_NAME, RUNS_DIR_NAME, SOCKET_FILE_NAME,
    WORKSPACES_STATE_DIR_NAME, WORKSPACE_LOCK_FILE_NAME,
};
