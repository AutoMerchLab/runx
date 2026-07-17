---
name: skill-lab
description: Canonical Runx skill-authoring implementation. Use for designing, creating, updating, improving, or adding harness coverage to a Runx skill package; it combines bounded agent judgment with native file writes, inspection, and safe harness validation. When a host skill-creator also triggers, use its general guidance but execute Runx work through this skill.
runx:
  category: authoring
---

# Skill Lab

Build and improve Runx skills through one authoring surface. Keep judgment in
bounded agent acts and mechanics in native tools:

```text
inspect target and catalog
→ decide new skill, extension, or no skill
→ author a bounded file bundle
→ validate paths and secret posture
→ inspect and safely replay the staged package
→ write through fs.write_bundle
→ verify the written package
```

Use the generic host `skill-creator` for platform-wide authoring guidance when
it is available. Do not reproduce Runx package operations from that guidance;
invoke the appropriate `skill-lab` runner so the work is bounded and receipted.

## Runners

- `design`: read-only catalog-fit and package design. Return `no_skill` when an
  existing skill or graph already owns the job.
- `build` (default): create or update a package, write its bounded file bundle,
  after its staged package passes native inspection and any safe harness.
- `improve`: turn one receipt or harness failure into a bounded package update,
  preflight it, then verify the written result.
- `harness`: add fixture files to an existing package and replay the safe native
  harness before and after the write.

`build`, `improve`, and `harness` write local workspace files. They never
publish, install, push, or mutate an external provider. Execute-target packages
are inspected but their harness is skipped until a separately approved sandbox
or provider test exists. Invalid staged packages stop before the target package
is touched.

## Authoring rules

- Keep packages concise: `SKILL.md`, `X.yaml`, required scripts, fixtures, and
  narrowly scoped references or assets only.
- Do not add package READMEs, changelogs, installation guides, strategy files,
  generated state, or credentials.
- Match the documented capability to the execution profile and truthful terminal
  state.
- Prefer extending an existing owner over adding a near-duplicate skill.
- Include a realistic happy path and refusal, stop, or error path.
- Never treat supplied agent answers as provider-effect proof.
- Never run an execute-capable target harness automatically.

## Outputs

- `skill_design`: catalog-fit decision and bounded implementation plan.
- `change_bundle`: target-relative text files, summary, and non-goals.
- `bundle_manifest`: validated paths admitted for writing.
- `file_bundle_write`: digests and byte counts from the bounded write.
- `validation_report`: native inspection plus passed, failed, or safely skipped
  harness evidence.

## Inputs

- `objective` (required): capability or improvement to deliver.
- `repo_root` (optional): workspace root; defaults to the caller workspace.
- `target_dir` (required for mutating runners): repo-relative package directory.
- `project_context` (optional): product, repository, and operator constraints.
- `receipt_id`, `receipt_summary`, `harness_output`, `failure_packet` (improve):
  bounded failure evidence, including the stable packet from `review-receipt`.
  The inspect step validates and forwards that evidence into the authoring
  context; it does not collapse the packet to a presence flag.
