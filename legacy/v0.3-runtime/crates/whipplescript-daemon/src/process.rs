use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use whipplescript_core::{WhippleScriptError, WhippleScriptResult};

pub fn spawn_shell_command(
    command: &str,
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    envs: &[(String, String)],
) -> WhippleScriptResult<Child> {
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
        .map_err(|error| WhippleScriptError::internal(format!("failed to spawn command: {error}")))
}

pub fn spawn_command(
    command: &[String],
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    envs: &[(String, String)],
) -> WhippleScriptResult<Child> {
    let Some((program, args)) = command.split_first() else {
        return Err(WhippleScriptError::invalid_input(
            "ad hoc command cannot be empty",
        ));
    };

    let stdout_file = File::create(stdout_path)?;
    let stderr_file = File::create(stderr_path)?;
    let mut child = Command::new(program);
    child
        .args(args)
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
        .map_err(|error| WhippleScriptError::internal(format!("failed to spawn command: {error}")))
}

pub fn signal_process_group(pid: u32, signal: i32) -> WhippleScriptResult<()> {
    let pgid = pid as i32;
    let result = unsafe { libc::kill(-pgid, signal) };
    if result == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(()),
        _ => Err(WhippleScriptError::internal(format!(
            "failed to signal process group {pgid}: {error}"
        ))),
    }
}
