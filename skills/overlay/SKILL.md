---
name: overlay
description: Turn a pinned local upstream SKILL.md into an exact native Runx binding bundle, with deterministic source digests, a bounded execution profile, isolated harness proof, and no publication claim. Use when adopting a third-party skill; use skill-lab for first-party Runx skills.
---

# Overlay

Runx executes governed upstream skills through native `bindings/<owner>/<skill>/`, not an independent overlay runtime.

## Procedure

1. Supply a workspace-local upstream `SKILL.md`, pinned upstream metadata, and registry metadata. The inspection step validates the path, frontmatter, commit and blob pins, URLs, and source-of-truth claim, then recomputes the file's SHA-256 and Git blob SHA.
2. Design one bounded agent-task profile from the upstream instructions. Return exact inputs, outputs, scopes, allowed tools, sandbox posture, and at least two mocked harness cases. Empty tool access is valid when the task needs no tools; it never means allow-all.
3. The deterministic finalizer rebuilds `X.yaml` as a native binding profile, stages it beside the unchanged upstream `SKILL.md`, runs native inspection and the isolated harness, and constructs `binding.json` from supplied evidence plus observed proof.
4. A ready result returns exact file contents and digests for `bindings/<owner>/<skill>/binding.json` and `X.yaml`. Apply that bundle through the owning repository's normal file-authoring lane, then run the existing binding materializer when publication is intended.

The skill does not fetch an unpinned registry ref, edit the upstream skill, write repository files, publish a package, or claim provider verification. Missing source evidence, digest drift, invalid profile shape, or failed harness proof stops before a bundle is released.

## Output

`binding_bundle` contains `decision`, `binding_path`, observed source evidence, exact `files`, native inspection and harness results, rationale, blockers, and a `success_checkpoint`. Only `decision: ready` contains files.

Inputs are `skill_path`, `upstream`, `registry`, optional `objective`, `scope_intent`, `tags`, and `publication`.
