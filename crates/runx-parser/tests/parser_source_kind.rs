use runx_parser::{
    InputMode, SourceKind, ValidateSkillOptions, parse_runner_manifest_yaml, parse_skill_markdown,
    validate_runner_manifest, validate_skill, validate_skill_with_options,
};

fn parse_strict(markdown: &str) -> Result<runx_parser::ValidatedSkill, String> {
    let raw = parse_skill_markdown(markdown).map_err(|error| error.to_string())?;
    validate_skill(raw).map_err(|error| error.to_string())
}

#[test]
fn cli_tool_source_parses_to_typed_kind_and_input_mode() -> Result<(), String> {
    let skill = parse_strict(
        r#"---
name: cli-skill
source:
  type: cli-tool
  command: node
  input_mode: stdin
---
# CLI
"#,
    )?;
    assert_eq!(skill.source.source_type, SourceKind::CliTool);
    assert_eq!(skill.source.input_mode, Some(InputMode::Stdin));
    // The typed kind serializes back to the original wire string.
    assert_eq!(skill.source.source_type.as_str(), "cli-tool");
    Ok(())
}

#[test]
fn javascript_source_parses_a_portable_module_without_process_plumbing() -> Result<(), String> {
    let skill = parse_strict(
        r#"---
name: javascript-skill
source:
  type: javascript
  module: domain/workflow.mjs
  export: execute
---
# JavaScript
"#,
    )?;
    assert_eq!(skill.source.source_type, SourceKind::JavaScript);
    assert_eq!(skill.source.module.as_deref(), Some("domain/workflow.mjs"));
    assert_eq!(skill.source.javascript_export.as_deref(), Some("execute"));
    assert_eq!(skill.source.command, None);
    assert_eq!(skill.source.source_type.as_str(), "javascript");
    assert_eq!(skill.source.sandbox, None);
    Ok(())
}

#[test]
fn javascript_source_parses_runtime_owned_artifact_pages() -> Result<(), String> {
    let skill = parse_strict(
        r#"---
name: paged-javascript
inputs:
  archive_file:
    type: string
    required: true
  archive_base:
    type: string
    required: true
source:
  type: javascript
  module: domain.mjs
  pages:
    path_from: archive_file
    path_scope_from: archive_base
    media_type: application/javascript
    framing: json_array
    page_bytes: 524288
---
# Paged JavaScript
"#,
    )?;
    let pages = skill
        .source
        .pages
        .ok_or_else(|| "artifact pages were not parsed".to_owned())?;
    assert_eq!(pages.path_from, "archive_file");
    assert_eq!(pages.path_scope_from.as_deref(), Some("archive_base"));
    assert_eq!(pages.page_bytes, 524_288);
    Ok(())
}

#[test]
fn javascript_source_rejects_ambiguous_or_reserved_page_inputs() -> Result<(), String> {
    for pages in [
        "path_from: runx_page\n      media_type: application/json\n      framing: json_array",
        "path_from: archive\n      path_scope_from: archive\n      media_type: application/json\n      framing: json_array",
        "path_from: archive\n      media_type: application/json\n      framing: json_array\n      page_bytes: 1048577",
    ] {
        let raw = parse_skill_markdown(&format!(
            "---\nname: bad-pages\nsource:\n  type: javascript\n  module: domain.mjs\n  pages:\n      {pages}\n---\n# Bad pages\n"
        ))
        .map_err(|error| error.to_string())?;
        assert!(
            validate_skill(raw).is_err(),
            "invalid pages unexpectedly passed"
        );
    }
    Ok(())
}

#[test]
fn javascript_source_rejects_process_fields_and_path_escape() -> Result<(), String> {
    for source in [
        "type: javascript\n  module: ../escape.mjs",
        "type: javascript\n  module: domain.mjs\n  command: node",
        "type: javascript\n  module: domain.mjs\n  args: [extra]",
        "type: javascript\n  module: domain.mjs\n  timeout_seconds: 1",
        "type: javascript\n  module: domain.mjs\n  export: not-valid",
        "type: cli-tool\n  command: node\n  module: domain.mjs",
    ] {
        let raw = parse_skill_markdown(&format!(
            "---\nname: bad-javascript\nsource:\n  {source}\n---\n# JavaScript\n"
        ))
        .map_err(|error| error.to_string())?;
        assert!(
            validate_skill(raw).is_err(),
            "invalid javascript source unexpectedly passed: {source}"
        );
    }
    Ok(())
}

#[test]
fn javascript_runner_rejects_runx_level_sandbox_controls() -> Result<(), String> {
    let raw = parse_runner_manifest_yaml(
        r#"skill: sandboxed-javascript
runners:
  transform:
    type: javascript
    module: domain.mjs
    runx:
      sandbox:
        profile: readonly
        network: false
"#,
    )
    .map_err(|error| error.to_string())?;
    let error = validate_runner_manifest(raw)
        .err()
        .ok_or_else(|| "runx-level javascript sandbox unexpectedly passed".to_owned())?;
    assert!(error.to_string().contains("cannot declare a sandbox"));
    Ok(())
}

#[test]
fn javascript_source_rejects_every_author_selected_sandbox() -> Result<(), String> {
    for sandbox in [
        "      profile: readonly\n      cwd_policy: skill-directory\n      network: false",
        "      profile: network\n      network: true",
        "      profile: workspace-write\n      writable_paths: [out]",
        "      profile: readonly\n      cwd_policy: workspace",
        "      profile: readonly\n      env_allowlist: [EXAMPLE_TOKEN]",
        "      profile: readonly\n      require_enforcement: false",
    ] {
        let raw = parse_skill_markdown(&format!(
            "---\nname: effectful-javascript\nsource:\n  type: javascript\n  module: domain.mjs\n  sandbox:\n{sandbox}\n---\n# JavaScript\n"
        ))
        .map_err(|error| error.to_string())?;
        let error = validate_skill(raw)
            .err()
            .ok_or_else(|| format!("effectful javascript sandbox passed: {sandbox}"))?;
        assert!(error.to_string().contains("cannot declare a sandbox"));
    }
    Ok(())
}

#[test]
fn javascript_runner_rejects_credential_delivery() -> Result<(), String> {
    let raw = parse_runner_manifest_yaml(
        r#"skill: credentialed-javascript
runners:
  transform:
    type: javascript
    module: domain.mjs
    credential: provider-user
"#,
    )
    .map_err(|error| error.to_string())?;
    let error = validate_runner_manifest(raw)
        .err()
        .ok_or_else(|| "credentialed javascript source unexpectedly passed".to_owned())?;
    assert!(
        error.to_string().contains("native provider tool"),
        "{error}"
    );
    Ok(())
}

#[test]
fn default_source_is_agent_kind() -> Result<(), String> {
    // A skill with no explicit source defaults to the `agent` source; the typed
    // `SourceKind` must carry an `Agent` variant for that (the built-in default).
    let raw = parse_skill_markdown(
        r#"---
name: portable-agent
inputs:
  prompt:
    type: string
    required: true
---
# Portable agent
"#,
    )
    .map_err(|error| error.to_string())?;
    let skill = validate_skill_with_options(raw, ValidateSkillOptions::lenient())
        .map_err(|error| error.to_string())?;
    assert_eq!(skill.source.source_type, SourceKind::Agent);
    Ok(())
}

#[test]
fn retired_http_source_points_to_native_http_tools() -> Result<(), String> {
    let raw = parse_skill_markdown(
        r#"---
name: retired-http-source
source:
  type: http
  url: https://api.example.test/v1/pets
---
# HTTP
"#,
    )
    .map_err(|error| error.to_string())?;
    let error = validate_skill(raw)
        .err()
        .ok_or_else(|| "retired http source unexpectedly validated".to_owned())?;
    assert!(error.to_string().contains(
        "source.type http was removed; compose http.read, http.query, or http.execute in a graph"
    ));
    Ok(())
}

#[test]
fn retired_catalog_source_points_to_graph_tool_steps() -> Result<(), String> {
    let raw = parse_skill_markdown(
        r#"---
name: retired-catalog-source
source:
  type: catalog
  catalog_ref: git.status
---
# Catalog
"#,
    )
    .map_err(|error| error.to_string())?;
    let error = validate_skill(raw)
        .err()
        .ok_or_else(|| "retired catalog source unexpectedly validated".to_owned())?;
    assert!(
        error.to_string().contains(
            "source.type catalog was removed; invoke catalog tools from a graph tool step"
        )
    );
    Ok(())
}

#[test]
fn thread_outbox_provider_source_parses_as_closed_builtin() -> Result<(), String> {
    let skill = parse_strict(
        r#"---
name: thread-outbox-provider-push
source:
  type: thread-outbox-provider
  thread_outbox_provider:
    operation: push
    manifest_path: manifest.json
    push_path: push.json
---
# Thread outbox provider
"#,
    )?;
    assert_eq!(skill.source.source_type, SourceKind::ThreadOutboxProvider);
    assert_eq!(skill.source.source_type.as_str(), "thread-outbox-provider");
    Ok(())
}

#[test]
fn unknown_source_type_fails_closed() -> Result<(), String> {
    let raw = parse_skill_markdown(
        r#"---
name: bogus
source:
  type: not-a-real-source
---
# Bogus
"#,
    )
    .map_err(|error| error.to_string())?;
    assert!(
        validate_skill(raw).is_err(),
        "an unknown source.type must fail closed at parse time"
    );
    Ok(())
}
