use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use armature_core::{ArmatureError, ArmatureResult};

pub fn spawn_shell_command(
    command: &str,
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    envs: &[(String, String)],
) -> ArmatureResult<Child> {
    let stdout_file = File::create(stdout_path)?;
    let stderr_file = File::create(stderr_path)?;
    let mut child = Command::new("sh");
    child
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    for (key, value) in envs {
        child.env(key, value);
    }

    unsafe {
        child.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    child
        .spawn()
        .map_err(|error| ArmatureError::internal(format!("failed to spawn command: {error}")))
}

pub fn signal_process_group(pid: u32, signal: i32) -> ArmatureResult<()> {
    let pgid = pid as i32;
    let result = unsafe { libc::kill(-pgid, signal) };
    if result == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(()),
        _ => Err(ArmatureError::internal(format!(
            "failed to signal process group {pgid}: {error}"
        ))),
    }
}
