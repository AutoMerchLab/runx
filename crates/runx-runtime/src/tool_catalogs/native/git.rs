//! Native bounded Git observations.

use std::time::Duration;

use super::{NativeInvocation, invalid_input, resolve_repo_root_for};
use crate::RuntimeError;

mod capability;

use crate::process::{ProcessOutcome, ProcessSpec, run_process};
use crate::services::SandboxServices;
pub(super) use capability::CAPABILITIES;
use capability::{
    GitBlobDigest, GitBlobDigestInput, GitBlobDigestOutput, GitBranchOutput, GitDiffInput,
    GitDiffOutput, GitInput, GitStatusOutput,
};

const CURRENT_BRANCH: &str = "git.current_branch";
const STATUS: &str = "git.status";
const DIFF_NAME_ONLY: &str = "git.diff_name_only";
const OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

fn blob_digest(
    invocation: &NativeInvocation<'_, GitBlobDigestInput>,
) -> Result<GitBlobDigestOutput, RuntimeError> {
    let contents = invocation.inputs.contents.as_bytes();
    let mut canonical = format!("blob {}\0", contents.len()).into_bytes();
    canonical.extend_from_slice(contents);
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &canonical);
    Ok(GitBlobDigestOutput {
        git_blob_digest: GitBlobDigest {
            algorithm: "sha1".to_owned(),
            digest: runx_contracts::hex_lower(digest.as_ref()),
            bytes: contents.len() as u64,
        },
    })
}

fn current_branch(
    invocation: &NativeInvocation<'_, GitInput>,
) -> Result<GitBranchOutput, RuntimeError> {
    let root = resolve_repo_root_for(
        CURRENT_BRANCH,
        &invocation.inputs.repo_root,
        invocation.env,
        invocation.skill_directory,
    )?;
    let env = git_env(invocation)?;
    let symbolic = run_git(&root, &env, &["symbolic-ref", "--short", "HEAD"])?;
    let (branch, detached) = if symbolic.status.success() {
        (output_text(CURRENT_BRANCH, invocation, symbolic)?, false)
    } else {
        let head = run_git(&root, &env, &["rev-parse", "--short", "HEAD"])?;
        if !head.status.success() {
            return Err(invalid_input(
                CURRENT_BRANCH,
                "repo_root must be a Git repository with a readable HEAD",
            ));
        }
        (output_text(CURRENT_BRANCH, invocation, head)?, true)
    };
    if branch.is_empty() {
        return Err(invalid_input(
            CURRENT_BRANCH,
            "Git returned an empty HEAD reference",
        ));
    }

    Ok(GitBranchOutput {
        repo_root: root.to_string_lossy().into_owned(),
        branch,
        detached,
    })
}

fn status(invocation: &NativeInvocation<'_, GitInput>) -> Result<GitStatusOutput, RuntimeError> {
    let root = resolve_repo_root_for(
        STATUS,
        &invocation.inputs.repo_root,
        invocation.env,
        invocation.skill_directory,
    )?;
    let env = git_env(invocation)?;
    let outcome = run_git(&root, &env, &["status", "--short", "--branch"])?;
    if !outcome.status.success() {
        return Err(invalid_input(
            STATUS,
            "repo_root must be a readable Git working tree",
        ));
    }
    let output = output_text(STATUS, invocation, outcome)?;
    let mut lines = output.lines();
    let first = lines.next();
    let (branch, entries) = match first {
        Some(line) if line.starts_with("## ") => (
            Some(line.trim_start_matches("## ").to_owned()),
            lines.map(str::to_owned).collect::<Vec<_>>(),
        ),
        Some(line) if !line.is_empty() => (
            None,
            std::iter::once(line.to_owned())
                .chain(lines.map(str::to_owned))
                .collect(),
        ),
        _ => (None, Vec::new()),
    };
    Ok(GitStatusOutput {
        repo_root: root.to_string_lossy().into_owned(),
        clean: entries.is_empty(),
        entries,
        branch,
    })
}

fn diff_name_only(
    invocation: &NativeInvocation<'_, GitDiffInput>,
) -> Result<GitDiffOutput, RuntimeError> {
    let root = resolve_repo_root_for(
        DIFF_NAME_ONLY,
        &invocation.inputs.repo_root,
        invocation.env,
        invocation.skill_directory,
    )?;
    let base = &invocation.inputs.base;
    validate_base(base)?;
    let env = git_env(invocation)?;
    let commitish = format!("{base}^{{commit}}");
    let resolved = run_git(&root, &env, &["rev-parse", "--verify", &commitish])?;
    if !resolved.status.success() {
        return Err(invalid_input(
            DIFF_NAME_ONLY,
            "base must resolve to a readable Git commit",
        ));
    }
    let commit = output_text(DIFF_NAME_ONLY, invocation, resolved)?;
    let outcome = run_git(
        &root,
        &env,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "--relative",
            &commit,
            "--",
        ],
    )?;
    if !outcome.status.success() {
        return Err(invalid_input(DIFF_NAME_ONLY, "Git diff failed"));
    }
    let files = output_text(DIFF_NAME_ONLY, invocation, outcome)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(GitDiffOutput {
        repo_root: root.to_string_lossy().into_owned(),
        base: base.to_owned(),
        files,
    })
}

fn validate_base(base: &str) -> Result<(), RuntimeError> {
    if base.is_empty()
        || base.starts_with('-')
        || base.len() > 1024
        || base.contains('\0')
        || base.chars().any(char::is_whitespace)
    {
        return Err(invalid_input(
            DIFF_NAME_ONLY,
            "base must be a bounded Git ref or commit id",
        ));
    }
    Ok(())
}

fn git_env<I: ?Sized>(
    invocation: &NativeInvocation<'_, I>,
) -> Result<std::collections::BTreeMap<String, String>, RuntimeError> {
    let mut env = SandboxServices.child_base_env(invocation.env)?;
    env.insert("GIT_OPTIONAL_LOCKS".to_owned(), "0".to_owned());
    env.insert("GIT_PAGER".to_owned(), "cat".to_owned());
    Ok(env)
}

fn run_git(
    root: &std::path::Path,
    env: &std::collections::BTreeMap<String, String>,
    args: &[&str],
) -> Result<ProcessOutcome, RuntimeError> {
    let mut bounded_args = vec![
        "--no-pager".to_owned(),
        "-c".to_owned(),
        "core.fsmonitor=false".to_owned(),
    ];
    bounded_args.extend(args.iter().map(|value| (*value).to_owned()));
    run_process(
        ProcessSpec::new("native Git observation", "git", OUTPUT_LIMIT_BYTES)
            .args(bounded_args)
            .cwd(root)
            .env(env.clone())
            .timeout(Some(TIMEOUT)),
    )
    .map_err(|error| invalid_input("git.observe", error.to_string()))
}

fn output_text<I: ?Sized>(
    tool: &str,
    invocation: &NativeInvocation<'_, I>,
    outcome: ProcessOutcome,
) -> Result<String, RuntimeError> {
    if outcome.timed_out || outcome.stdout.truncated || outcome.stderr.truncated {
        return Err(invalid_input(
            tool,
            "Git observation exceeded runtime bounds",
        ));
    }
    Ok(invocation
        .credential_delivery
        .redact_bytes_to_string(outcome.stdout.bytes, OUTPUT_LIMIT_BYTES)
        .trim()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::process::Command;

    use runx_contracts::{JsonObject, JsonValue};

    use super::{
        GitBlobDigestInput, GitDiffInput, GitInput, blob_digest, current_branch, diff_name_only,
        status,
    };
    #[cfg(feature = "catalog")]
    use crate::RuntimeEffectRegistry;
    use crate::credentials::CredentialDelivery;
    use crate::receipts::paths::RUNX_CWD_ENV;
    use crate::tool_catalogs::native::NativeInvocation;

    #[test]
    fn reads_named_and_detached_head() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        git(workspace.path(), &["init", "-b", "main"])?;
        git(
            workspace.path(),
            &["config", "user.email", "runx@example.invalid"],
        )?;
        git(workspace.path(), &["config", "user.name", "Runx Test"])?;
        std::fs::write(workspace.path().join("README.md"), "# Fixture\n")?;
        git(workspace.path(), &["add", "README.md"])?;
        git(workspace.path(), &["commit", "-m", "fixture"])?;

        let named = invoke_current_branch(workspace.path())?;
        assert_eq!(
            named.get("branch"),
            Some(&JsonValue::String("main".to_owned()))
        );
        assert_eq!(named.get("detached"), Some(&JsonValue::Bool(false)));

        git(workspace.path(), &["checkout", "--detach", "HEAD"])?;
        let detached = invoke_current_branch(workspace.path())?;
        assert_eq!(detached.get("detached"), Some(&JsonValue::Bool(true)));
        assert_eq!(
            detached
                .get("branch")
                .and_then(JsonValue::as_str)
                .map(str::len),
            Some(7)
        );
        Ok(())
    }

    #[test]
    fn reports_status_and_changed_files_without_local_wrappers()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        git(workspace.path(), &["init", "-b", "main"])?;
        git(
            workspace.path(),
            &["config", "user.email", "runx@example.invalid"],
        )?;
        git(workspace.path(), &["config", "user.name", "Runx Test"])?;
        std::fs::write(workspace.path().join("README.md"), "original\n")?;
        git(workspace.path(), &["add", "README.md"])?;
        git(workspace.path(), &["commit", "-m", "fixture"])?;
        std::fs::write(workspace.path().join("README.md"), "changed\n")?;

        let status = invoke_tool(
            workspace.path(),
            GitInput {
                repo_root: ".".to_owned(),
            },
            status,
        )?;
        assert_eq!(status.get("clean"), Some(&JsonValue::Bool(false)));
        assert_eq!(
            status.get("entries"),
            Some(&JsonValue::Array(vec![JsonValue::String(
                " M README.md".to_owned()
            )]))
        );

        let diff = invoke_tool(
            workspace.path(),
            GitDiffInput {
                repo_root: ".".to_owned(),
                base: "HEAD".to_owned(),
            },
            diff_name_only,
        )?;
        assert_eq!(
            diff.get("files"),
            Some(&JsonValue::Array(vec![JsonValue::String(
                "README.md".to_owned()
            )]))
        );
        Ok(())
    }

    #[test]
    fn computes_the_canonical_git_blob_identity_without_a_process()
    -> Result<(), Box<dyn std::error::Error>> {
        let workspace = tempfile::tempdir()?;
        let result = invoke_tool(
            workspace.path(),
            GitBlobDigestInput {
                contents: "hello\n".to_owned(),
            },
            blob_digest,
        )?;
        let digest = result
            .get("git_blob_digest")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| std::io::Error::other("missing git blob digest"))?;
        assert_eq!(
            digest.get("digest"),
            Some(&JsonValue::String(
                "ce013625030ba8dba906f756967f9e9ca394464a".to_owned()
            ))
        );
        assert_eq!(
            digest.get("bytes"),
            Some(&JsonValue::Number(runx_contracts::JsonNumber::U64(6)))
        );
        Ok(())
    }

    fn invoke_current_branch(
        root: &std::path::Path,
    ) -> Result<JsonObject, Box<dyn std::error::Error>> {
        invoke_tool(
            root,
            GitInput {
                repo_root: ".".to_owned(),
            },
            current_branch,
        )
    }

    fn invoke_tool<I, O: serde::Serialize>(
        root: &std::path::Path,
        inputs: I,
        tool: for<'a> fn(&NativeInvocation<'a, I>) -> Result<O, crate::RuntimeError>,
    ) -> Result<JsonObject, Box<dyn std::error::Error>> {
        let env = BTreeMap::from([(RUNX_CWD_ENV.to_owned(), root.to_string_lossy().into_owned())]);
        let delivery = CredentialDelivery::none();
        #[cfg(feature = "catalog")]
        let effects = RuntimeEffectRegistry::default();
        let output = tool(&NativeInvocation {
            inputs: &inputs,
            observed_at: "2026-01-01T00:00:00Z",
            data_source_binding: None,
            env: &env,
            skill_directory: root,
            credential_delivery: &delivery,
            local_artifacts: crate::tool_catalogs::native::fixture_local_artifacts(),
            #[cfg(feature = "catalog")]
            effects: &effects,
        })?;
        let value: JsonValue = serde_json::from_value(serde_json::to_value(output)?)?;
        value
            .as_object()
            .cloned()
            .ok_or_else(|| "missing output".into())
    }

    fn git(root: &std::path::Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()?;
        if !status.success() {
            return Err(format!("git command failed: {args:?}").into());
        }
        Ok(())
    }
}
