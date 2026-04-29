pub mod config;
pub mod error;
pub mod ids;
pub mod model;

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
