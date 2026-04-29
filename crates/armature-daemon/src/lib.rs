pub mod store;

use armature_core::{ArmatureError, ArmatureResult, RuntimeSnapshot};

#[derive(Debug, Default)]
pub struct Daemon {
    runtime: RuntimeSnapshot,
}

impl Daemon {
    pub fn new(runtime: RuntimeSnapshot) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &RuntimeSnapshot {
        &self.runtime
    }

    pub fn bootstrap() -> ArmatureResult<Self> {
        Ok(Self::default())
    }

    pub fn unimplemented(feature: &'static str) -> ArmatureError {
        ArmatureError::not_implemented(format!("{feature} is not implemented yet"))
    }
}

#[cfg(test)]
mod tests {
    use super::Daemon;

    #[test]
    fn bootstrap_provides_empty_runtime_shell() {
        let daemon = Daemon::bootstrap().unwrap();
        assert!(daemon.runtime().tasks.is_empty());
        assert!(daemon.runtime().services.is_empty());
        assert!(daemon.runtime().active_runs.is_empty());
    }
}
