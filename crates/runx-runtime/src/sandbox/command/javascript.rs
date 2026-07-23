use std::path::Path;

use crate::RuntimeError;

use super::super::backend::SandboxRuntime;
#[cfg(target_os = "linux")]
use super::path_string;
#[cfg(target_os = "macos")]
use super::sandbox_profile_string;

pub(in crate::sandbox) fn javascript_worker_spawn_command(
    runtime: Option<&SandboxRuntime>,
    worker_path: &Path,
    _cwd: &Path,
) -> Result<(String, Vec<String>), RuntimeError> {
    match runtime {
        #[cfg(target_os = "linux")]
        Some(SandboxRuntime::Bubblewrap { path }) => Ok((
            path.to_string_lossy().into_owned(),
            bubblewrap_args(worker_path),
        )),
        #[cfg(target_os = "macos")]
        Some(SandboxRuntime::SandboxExec { path }) => Ok((
            path.to_string_lossy().into_owned(),
            vec![
                "-p".to_owned(),
                sandbox_exec_profile(worker_path),
                worker_path.to_string_lossy().into_owned(),
            ],
        )),
        #[cfg(target_os = "windows")]
        Some(SandboxRuntime::Direct | SandboxRuntime::DeclaredPolicyOnly { .. }) | None => {
            Ok((worker_path.to_string_lossy().into_owned(), Vec::new()))
        }
        _ => Err(RuntimeError::SandboxViolation {
            message: "deterministic JavaScript worker requires a real platform sandbox backend"
                .to_owned(),
        }),
    }
}

#[cfg(target_os = "linux")]
fn bubblewrap_args(worker_path: &Path) -> Vec<String> {
    let mut args = vec![
        "--unshare-all".to_owned(),
        "--die-with-parent".to_owned(),
        "--proc".to_owned(),
        "/proc".to_owned(),
        "--dev".to_owned(),
        "/dev".to_owned(),
        "--tmpfs".to_owned(),
        "/tmp".to_owned(),
    ];
    for path in [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/etc/ld.so.cache",
    ] {
        args.extend(["--ro-bind-try".to_owned(), path.to_owned(), path.to_owned()]);
    }
    args.extend([
        "--ro-bind".to_owned(),
        path_string(worker_path),
        path_string(worker_path),
        "--chdir".to_owned(),
        "/tmp".to_owned(),
        "--".to_owned(),
        path_string(worker_path),
    ]);
    args
}

#[cfg(target_os = "macos")]
fn sandbox_exec_profile(worker_path: &Path) -> String {
    let worker = sandbox_profile_string(worker_path);
    format!(
        "(version 1)\n(deny default)\n(allow process-exec (literal \"{worker}\"))\n(allow sysctl-read)\n(allow file-read-data (literal \"/\"))\n(allow file-read* (subpath \"/System\") (subpath \"/usr\") (literal \"{worker}\"))"
    )
}
