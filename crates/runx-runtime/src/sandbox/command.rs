// Module rationale: sandbox command assembly keeps backend
// argument construction and mount-shape tests colocated with the policy it emits.
use std::fs;
use std::path::{Path, PathBuf};

use runx_core::policy::SandboxProfile;
use runx_parser::SkillSandbox;

use super::backend::SandboxRuntime;
use super::policy::normalize_path;

mod javascript;

pub(super) use javascript::javascript_worker_spawn_command;

pub(super) struct SandboxSpawnCommand<'a> {
    pub(super) runtime: Option<&'a SandboxRuntime>,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) cwd: &'a Path,
    pub(super) skill_directory: &'a Path,
    pub(super) workspace_cwd: Option<&'a Path>,
    pub(super) writable_paths: &'a [PathBuf],
    pub(super) network: bool,
    pub(super) private_tmp: Option<&'a Path>,
}

pub(super) fn sandbox_spawn_command(input: SandboxSpawnCommand<'_>) -> (String, Vec<String>) {
    match input.runtime {
        Some(SandboxRuntime::Bubblewrap { path }) => (
            path.to_string_lossy().into_owned(),
            bubblewrap_args(BubblewrapCommand {
                command: input.command,
                command_args: input.args,
                cwd: input.cwd,
                skill_directory: input.skill_directory,
                workspace_cwd: input.workspace_cwd,
                writable_paths: input.writable_paths,
                network: input.network,
                private_tmp: input.private_tmp,
            }),
        ),
        Some(SandboxRuntime::SandboxExec { path }) => (
            path.to_string_lossy().into_owned(),
            sandbox_exec_args(
                input.command,
                input.args,
                input.cwd,
                input.writable_paths,
                input.network,
                input.private_tmp,
            ),
        ),
        Some(SandboxRuntime::Direct | SandboxRuntime::DeclaredPolicyOnly { .. }) | None => {
            (input.command, input.args)
        }
    }
}

pub(super) fn sandbox_network_enabled(sandbox: Option<&SkillSandbox>) -> bool {
    sandbox.is_some_and(|sandbox| {
        sandbox
            .network
            .unwrap_or(sandbox.profile == SandboxProfile::Network)
    })
}

struct BubblewrapCommand<'a> {
    command: String,
    command_args: Vec<String>,
    cwd: &'a Path,
    skill_directory: &'a Path,
    workspace_cwd: Option<&'a Path>,
    writable_paths: &'a [PathBuf],
    network: bool,
    private_tmp: Option<&'a Path>,
}

fn bubblewrap_args(input: BubblewrapCommand<'_>) -> Vec<String> {
    let BubblewrapCommand {
        command,
        command_args,
        cwd,
        skill_directory,
        workspace_cwd,
        writable_paths,
        network,
        private_tmp,
    } = input;
    let workspace_root = workspace_cwd.map(normalize_path).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|path| normalize_path(&path))
    });
    let mut args = vec!["--unshare-all".to_owned()];
    if network {
        args.push("--share-net".to_owned());
    }
    args.extend([
        "--die-with-parent".to_owned(),
        "--proc".to_owned(),
        "/proc".to_owned(),
        "--dev".to_owned(),
        "/dev".to_owned(),
        "--tmpfs".to_owned(),
        "/tmp".to_owned(),
    ]);
    for mount_path in readonly_mounts(skill_directory, workspace_root.as_deref(), cwd, network) {
        args.extend([
            "--ro-bind-try".to_owned(),
            path_string(&mount_path),
            path_string(&mount_path),
        ]);
    }
    if let Some(private_tmp) = private_tmp {
        args.extend([
            "--bind".to_owned(),
            path_string(private_tmp),
            path_string(private_tmp),
        ]);
    }
    for mount in writable_mounts(writable_paths, cwd) {
        args.extend([
            "--bind".to_owned(),
            path_string(&mount),
            path_string(&mount),
        ]);
    }
    args.extend([
        "--chdir".to_owned(),
        path_string(cwd),
        "--".to_owned(),
        command,
    ]);
    args.extend(command_args);
    args
}

fn readonly_mounts(
    skill_directory: &Path,
    workspace_root: Option<&Path>,
    cwd: &Path,
    network: bool,
) -> Vec<PathBuf> {
    unique_paths(
        system_readonly_mounts(network)
            .into_iter()
            .chain(find_package_root(skill_directory))
            .chain([normalize_existing_path(skill_directory)])
            .chain(workspace_root.map(Path::to_path_buf))
            .chain([normalize_existing_path(cwd)])
            .collect(),
    )
}

fn system_readonly_mounts(network: bool) -> Vec<PathBuf> {
    let mut mounts = super::LINUX_RUNTIME_READONLY_PATHS
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if network {
        mounts.extend(
            [
                "/etc/hosts",
                "/etc/resolv.conf",
                "/etc/nsswitch.conf",
                "/etc/ssl",
                "/etc/pki",
                "/etc/ca-certificates",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
    }
    mounts
}

fn writable_mounts(writable_paths: &[PathBuf], cwd: &Path) -> Vec<PathBuf> {
    unique_paths(
        writable_paths
            .iter()
            .map(|path| writable_mount_path(path, cwd))
            .collect(),
    )
}

fn writable_mount_path(path: &Path, cwd: &Path) -> PathBuf {
    let path = resolve_pathbuf(cwd, path);
    if path.exists() {
        return normalize_existing_path(&path);
    }
    normalize_path(&path)
}

fn sandbox_exec_args(
    command: String,
    command_args: Vec<String>,
    cwd: &Path,
    writable_paths: &[PathBuf],
    network: bool,
    private_tmp: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_owned(),
        sandbox_exec_profile(cwd, writable_paths, network, private_tmp),
    ];
    args.push(command);
    args.extend(command_args);
    args
}

pub(super) fn sandbox_exec_profile(
    cwd: &Path,
    writable_paths: &[PathBuf],
    network: bool,
    private_tmp: Option<&Path>,
) -> String {
    let mut profile = [
        "(version 1)",
        "(deny default)",
        "(allow process*)",
        "(allow sysctl*)",
        "(allow file-read*)",
        "(allow file-write* (literal \"/dev/null\"))",
    ]
    .join("\n");
    if network {
        profile.push_str("\n(allow network*)");
        profile.push_str("\n(allow mach-lookup)");
    }
    for writable_path in writable_paths {
        let declared = resolve_pathbuf(cwd, writable_path);
        let path = sandbox_exec_path_filter_path(&declared);
        if declared.is_dir() {
            profile.push_str(&format!(
                "\n(allow file-write* (literal \"{}\") (subpath \"{}\"))",
                sandbox_profile_string(&path),
                sandbox_profile_string(&path)
            ));
        } else {
            profile.push_str(&format!(
                "\n(allow file-write* (literal \"{}\"))",
                sandbox_profile_string(&path)
            ));
        }
    }
    if let Some(private_tmp) = private_tmp {
        let path = sandbox_exec_path_filter_path(private_tmp);
        profile.push_str(&format!(
            "\n(allow file-write* (literal \"{}\") (subpath \"{}\"))",
            sandbox_profile_string(&path),
            sandbox_profile_string(&path)
        ));
    }
    profile
}

pub(super) fn sandbox_exec_path_filter_path(path: &Path) -> PathBuf {
    if path.exists() {
        return normalize_existing_path(path);
    }
    let parent = path.parent().map(normalize_existing_path);
    parent
        .map(|parent| {
            path.file_name()
                .map(|name| parent.join(name))
                .unwrap_or(parent)
        })
        .unwrap_or_else(|| path.to_path_buf())
}

fn resolve_pathbuf(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

pub(super) fn sandbox_profile_string(path: &Path) -> String {
    path_string(path)
        .chars()
        .map(|character| match character {
            '\\' => "\\\\".to_owned(),
            '"' => "\\\"".to_owned(),
            '(' | ')' | ';' => "_".to_owned(),
            character if character.is_control() => "_".to_owned(),
            character => character.to_string(),
        })
        .collect()
}

fn find_package_root(start: &Path) -> Option<PathBuf> {
    let mut current = normalize_existing_path(start);
    let mut found = None;
    loop {
        if current.join("package.json").exists() || current.join("pnpm-workspace.yaml").exists() {
            found = Some(current.clone());
        }
        let Some(parent) = current.parent() else {
            return found;
        };
        if parent == current {
            return found;
        }
        current = parent.to_path_buf();
    }
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|prior| prior == &path) {
            unique.push(path);
        }
    }
    unique
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::{BubblewrapCommand, bubblewrap_args, system_readonly_mounts};
    use std::path::{Path, PathBuf};

    #[test]
    fn bubblewrap_readonly_mounts_do_not_expose_broad_system_config() {
        let mounts = system_readonly_mounts(false);

        assert!(mounts.contains(&PathBuf::from("/usr")));
        assert!(mounts.contains(&PathBuf::from("/lib")));
        assert!(mounts.contains(&PathBuf::from("/etc/ld.so.cache")));
        assert!(!mounts.contains(&PathBuf::from("/etc")));
        assert!(!mounts.contains(&PathBuf::from("/opt")));
        assert!(!mounts.contains(&PathBuf::from("/nix")));
        assert!(!mounts.contains(&PathBuf::from("/snap")));
    }

    #[test]
    fn bubblewrap_network_mounts_only_curated_etc_paths() {
        let mounts = system_readonly_mounts(true);

        assert!(!mounts.contains(&PathBuf::from("/etc")));
        assert!(mounts.contains(&PathBuf::from("/etc/resolv.conf")));
        assert!(mounts.contains(&PathBuf::from("/etc/hosts")));
        assert!(mounts.contains(&PathBuf::from("/etc/ssl")));
    }

    #[test]
    fn bubblewrap_keeps_non_existing_writable_file_grants_narrow() {
        let args = bubblewrap_args(BubblewrapCommand {
            command: "tool".to_owned(),
            command_args: Vec::new(),
            cwd: Path::new("/workspace/skill"),
            skill_directory: Path::new("/workspace/skill"),
            workspace_cwd: Some(Path::new("/workspace")),
            writable_paths: &[PathBuf::from("/workspace/logs/output.json")],
            network: false,
            private_tmp: None,
        });

        assert!(args.windows(3).any(|window| {
            window
                == [
                    "--bind".to_owned(),
                    "/workspace/logs/output.json".to_owned(),
                    "/workspace/logs/output.json".to_owned(),
                ]
        }));
    }
}
