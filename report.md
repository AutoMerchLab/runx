# CRM Cleanup Skill - Delivery Report

## Overview
This report documents the published `crm-cleanup` runx skill and a real, verifiable
post-publish dogfood run.

## Package
- **Skill**: `crm-cleanup` | **Owner**: `automerchlab` | **Version**: `sha-0c7de8ffc412`
- **Registry ref**: `automerchlab/crm-cleanup@sha-0c7de8ffc412`
- **public_url**: https://runx.ai/x/automerchlab/crm-cleanup@sha-0c7de8ffc412
- **pr_url**: https://github.com/runxhq/runx/pull/264
- **source_url**: https://github.com/automerchlab/runx/tree/e2cb3b4678fd123c35d4851cd136f1d13f805ae5
- **raw X.yaml**: https://raw.githubusercontent.com/automerchlab/runx/e2cb3b4678fd123c35d4851cd136f1d13f805ae5/skills/crm-cleanup/X.yaml
- **raw SKILL.md**: https://raw.githubusercontent.com/automerchlab/runx/e2cb3b4678fd123c35d4851cd136f1d13f805ae5/skills/crm-cleanup/SKILL.md

## runx CLI
- `runx --version` -> **runx-cli 0.6.16** (>= 0.6.14 floor). Used for install, dogfood, and verify.

## Install (clean)
- `runx add automerchlab/crm-cleanup@sha-0c7de8ffc412 --registry https://api.runx.ai` -> source=remote, status=installed.

## Harness
- Local harness: `runx harness ./skills/crm-cleanup` -> **3/3 PASSED, 0 assertion errors** (WSL Linux).
- Cases: **actionable** (sealed - takeaways + field_updates + gated write_proposal), **noop** (sealed - empty field_updates, no-op), **invalid_schema** (failed - refuses on empty schema).

## Dogfood (post-publish, real)
- Command: `runx skill automerchlab/crm-cleanup@sha-0c7de8ffc412 --registry https://api.runx.ai --json -i transcript='The client said the budget is $50k. The status is qualified. The next step is follow up call.' -i crm_schema='{"fields": ["budget", "status", "next_step"]}' -R ./receipts`
- Output: takeaways Identified budget: $50k; Identified status: qualified; Identified next step: follow up call; field_updates {budget=$50k, next_step=follow up call, status=qualified}; write_proposal true.
- Receipt: `runx:receipt:sha256:f4030faf730b22a3046d77e3bdb4649e73e6a4e383d8e6c4c702dc4ca4fd9c89`
- `runx verify --receipt dogfood_receipt.json --json` -> **valid: true, signature_mode: production, signature: valid**.

## Provenance (single source revision)
- Registry provenance (from the dogfood receipt): registry_source=remote https://api.runx.ai, skill_id=automerchlab/crm-cleanup, version=sha-0c7de8ffc412, trust_state=trusted, trust_tier=community - the dogfood run
  resolved the published package from the remote registry at the exact published version.
- source_url, raw X.yaml, raw SKILL.md and verification.json all resolve at one source revision:
  commit `e2cb3b4678fd123c35d4851cd136f1d13f805ae5` on the `automerchlab/runx` `crm-cleanup` branch.
- The skill files at `e2cb3b4678fd123c35d4851cd136f1d13f805ae5` are byte-identical to the published package `automerchlab/crm-cleanup@sha-0c7de8ffc412` (matching digest).
- This report and evidence.json are committed as the direct child of `e2cb3b4678fd123c35d4851cd136f1d13f805ae5` and describe that same
  revision; the recorded receipt_ref is the post-publish dogfood run of the published package, not a
  harness fixture seal.

## What to inspect first
1. `runx verify --receipt dogfood_receipt.json --json` (valid=true, production).
2. `evidence.json` dogfood.output (real takeaways + field_updates + gated write_proposal).
3. Raw X.yaml / SKILL.md / verification.json at source revision `e2cb3b4678fd123c35d4851cd136f1d13f805ae5`.
