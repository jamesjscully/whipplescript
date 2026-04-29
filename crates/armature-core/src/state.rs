use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{ArmatureError, ArmatureResult, RunId, Workspace, WorkspaceId};

pub const DEFAULT_STATE_HOME_SUFFIX: &str = ".local/state";
pub const ARMATURE_STATE_DIR_NAME: &str = "armature";
pub const WORKSPACES_STATE_DIR_NAME: &str = "workspaces";
pub const RUNS_DIR_NAME: &str = "runs";
pub const DATABASE_FILE_NAME: &str = "armature.sqlite";
pub const SOCKET_FILE_NAME: &str = "daemon.sock";
pub const PID_FILE_NAME: &str = "daemon.pid";
pub const WORKSPACE_LOCK_FILE_NAME: &str = "workspace.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRuntimePaths {
    workspace_id: WorkspaceId,
    workspace_root: PathBuf,
    state_root: PathBuf,
    runs_root: PathBuf,
}

impl WorkspaceRuntimePaths {
    pub fn for_workspace(workspace: &Workspace) -> ArmatureResult<Self> {
        let state_home = state_home_from_env()?;
        Self::for_workspace_with_state_home(workspace, state_home)
    }

    pub fn for_workspace_with_state_home(
        workspace: &Workspace,
        state_home: impl AsRef<Path>,
    ) -> ArmatureResult<Self> {
        let workspace_root = workspace.root().canonicalize()?;
        let workspace_id = WorkspaceId::from_canonical_path(&workspace_root)?;
        let state_root = state_home
            .as_ref()
            .join(ARMATURE_STATE_DIR_NAME)
            .join(WORKSPACES_STATE_DIR_NAME)
            .join(workspace_id.as_str());
        let runs_root = workspace.config_dir().join(RUNS_DIR_NAME);

        Ok(Self {
            workspace_id,
            workspace_root,
            state_root,
            runs_root,
        })
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn runs_root(&self) -> &Path {
        &self.runs_root
    }

    pub fn database_path(&self) -> PathBuf {
        self.state_root.join(DATABASE_FILE_NAME)
    }

    pub fn socket_path(&self) -> PathBuf {
        self.state_root.join(SOCKET_FILE_NAME)
    }

    pub fn pid_path(&self) -> PathBuf {
        self.state_root.join(PID_FILE_NAME)
    }

    pub fn workspace_lock_path(&self) -> PathBuf {
        self.state_root.join(WORKSPACE_LOCK_FILE_NAME)
    }

    pub fn run_paths(&self, run_id: &RunId) -> RunPaths {
        let directory = self.runs_root.join(run_id.as_str());

        RunPaths {
            directory: directory.clone(),
            stdout: directory.join("stdout.log"),
            stderr: directory.join("stderr.log"),
            meta: directory.join("meta.json"),
            event: directory.join("event.json"),
            tmp: directory.join("tmp"),
        }
    }

    pub fn ensure_state_root(&self) -> ArmatureResult<()> {
        fs::create_dir_all(&self.state_root)?;
        Ok(())
    }

    pub fn prepare_run_directory(&self, run_id: &RunId) -> ArmatureResult<RunPaths> {
        let paths = self.run_paths(run_id);
        fs::create_dir_all(&paths.tmp)?;
        Ok(paths)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPaths {
    pub directory: PathBuf,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
    pub meta: PathBuf,
    pub event: PathBuf,
    pub tmp: PathBuf,
}

pub fn state_home_from_env() -> ArmatureResult<PathBuf> {
    resolve_state_home(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn resolve_state_home(
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> ArmatureResult<PathBuf> {
    if let Some(path) = xdg_state_home.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let home = home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            ArmatureError::invalid_input(
                "unable to resolve Armature state home: set XDG_STATE_HOME or HOME",
            )
        })?;

    Ok(home.join(DEFAULT_STATE_HOME_SUFFIX))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{
        resolve_state_home, WorkspaceRuntimePaths, ARMATURE_STATE_DIR_NAME, DATABASE_FILE_NAME,
        WORKSPACES_STATE_DIR_NAME,
    };
    use crate::discover_workspace;

    #[test]
    fn uses_xdg_state_home_when_available() {
        let state_home = resolve_state_home(
            Some(PathBuf::from("/tmp/armature-state").into_os_string()),
            None,
        )
        .unwrap();
        assert_eq!(state_home, PathBuf::from("/tmp/armature-state"));
    }

    #[test]
    fn falls_back_to_local_state_under_home() {
        let state_home =
            resolve_state_home(None, Some(PathBuf::from("/home/alice").into_os_string())).unwrap();
        assert_eq!(state_home, PathBuf::from("/home/alice/.local/state"));
    }

    #[test]
    fn runtime_paths_separate_state_root_from_run_artifacts() {
        let fixture = WorkspaceFixture::new();
        let workspace = discover_workspace(fixture.root()).unwrap();
        let paths = WorkspaceRuntimePaths::for_workspace_with_state_home(
            &workspace,
            fixture.state_home.path(),
        )
        .unwrap();

        assert_eq!(paths.workspace_root(), fixture.root());
        assert!(
            paths.state_root().starts_with(fixture.state_home.path()),
            "state root should live under the configured state home"
        );
        assert_eq!(
            paths.database_path(),
            paths.state_root().join(DATABASE_FILE_NAME)
        );
        assert_eq!(
            paths.runs_root(),
            &fixture.root().join(".armature").join("runs")
        );
        assert!(
            paths.state_root().ends_with(paths.workspace_id().as_str()),
            "workspace state root should be keyed by the stable workspace id"
        );
        assert!(paths.state_root().components().any(|component| {
            component.as_os_str() == ARMATURE_STATE_DIR_NAME
                || component.as_os_str() == WORKSPACES_STATE_DIR_NAME
        }));
    }

    struct WorkspaceFixture {
        root_dir: TempDir,
        state_home: TempDir,
    }

    impl WorkspaceFixture {
        fn new() -> Self {
            let root_dir = TempDir::new().unwrap();
            fs::create_dir_all(root_dir.path().join(".armature")).unwrap();
            fs::write(
                root_dir.path().join(".armature/armature.toml"),
                "[[task]]\nname = \"demo\"\nrun = \"true\"\n",
            )
            .unwrap();

            Self {
                root_dir,
                state_home: TempDir::new().unwrap(),
            }
        }

        fn root(&self) -> &Path {
            self.root_dir.path()
        }
    }
}
