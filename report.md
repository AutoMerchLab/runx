# CRM Cleanup Skill - Delivery Report

## Overview
This report documents the published `crm-cleanup` runx skill and a real, verifiable
post-publish dogfood run. This revision answers the prior review directly:
- extraction now handles **natural phrasings**, not only the literal "field is value" template
  (field-type inference over money / status / next-step / generic + synonym vocab);
- lines that reference a field with an actionable cue but carry no extractable value are
  emitted under **`needs_review`** `{field, line, reason}` instead of being silently dropped;
- **typed outputs** (`takeaways`, `field_updates`, `needs_review`, `write_proposal`) are now
  declared in `X.yaml`;
- the description is scoped to what actually ships (deterministic heuristics, not "dynamic NLP").

## Package
- **Skill**: `crm-cleanup` | **Owner**: `automerchlab` | **Version**: `sha-44376c7817e7`
- **Registry ref**: `automerchlab/crm-cleanup@sha-44376c7817e7`
- **public_url**: https://runx.ai/x/automerchlab/crm-cleanup@sha-44376c7817e7
- **pr_url**: https://github.com/runxhq/runx/pull/264
- **source_url**: https://github.com/automerchlab/runx/tree/3266c6b6c8d830e5f05447d93e9a1e3011ccb056
- **raw X.yaml**: https://raw.githubusercontent.com/automerchlab/runx/3266c6b6c8d830e5f05447d93e9a1e3011ccb056/skills/crm-cleanup/X.yaml
- **raw SKILL.md**: https://raw.githubusercontent.com/automerchlab/runx/3266c6b6c8d830e5f05447d93e9a1e3011ccb056/skills/crm-cleanup/SKILL.md

## runx CLI
- `runx --version` -> **runx-cli 0.6.16** (>= 0.6.14 floor). Used for install, dogfood, and verify.

## Install (clean)
- `runx add automerchlab/crm-cleanup@sha-44376c7817e7 --registry https://api.runx.ai` -> source=remote, status=installed.

## Harness
- Local harness: `runx harness ./skills/crm-cleanup` -> **5/5 PASSED, 0 assertion errors** (WSL Linux).
- Cases: templated (sealed), natural (sealed), needs_review (sealed), noop (sealed), invalid_schema (failed).
  - **templated** - literal "field is value" phrasing (backward compatible).
  - **natural** - phrasing NOT shaped to the regex (`got around $75k`, `moving them to qualified`, `send over a proposal`).
  - **needs_review** - fields referenced with a cue but no extractable value -> reported in `needs_review`, not dropped.
  - **noop** - no actionable change -> empty field_updates, write_proposal false.
  - **invalid_schema** - refuses on an empty/malformed schema.

## Dogfood (post-publish, real, natural transcript)
- Command: `runx skill automerchlab/crm-cleanup@sha-44376c7817e7 --registry https://api.runx.ai --json -i transcript='Great call. They have got around $75k to spend this quarter, so I am moving them to qualified. I will send over a proposal by Friday.' -i crm_schema='{"fields": ["budget", "status", "next_step"]}' -R ./receipts`
- Output: takeaways Identified budget: $75k; Identified status: qualified; Identified next step: send over a proposal by Friday; field_updates {budget=$75k, next_step=send over a proposal by Friday, status=qualified}; needs_review []; write_proposal true.
- Receipt: `runx:receipt:sha256:0ea8bb73fca87c1963a160120fe64dbb753a12809fce3d1db13d6a27f7ab4e36`
- `runx verify --receipt dogfood_receipt.json --json` -> **valid: true, signature_mode: production, signature: valid**.

## Provenance (single source revision)
- Registry provenance (from the dogfood receipt): registry_source=remote https://api.runx.ai, skill_id=automerchlab/crm-cleanup, version=sha-44376c7817e7, trust_state=trusted, trust_tier=community - the dogfood run
  resolved the published package from the remote registry at the exact published version.
- source_url, raw X.yaml, raw SKILL.md and verification.json all resolve at one source revision:
  commit `3266c6b6c8d830e5f05447d93e9a1e3011ccb056` on the `automerchlab/runx` `crm-cleanup` branch.
- The skill files at `3266c6b6c8d830e5f05447d93e9a1e3011ccb056` are byte-identical to the published package `automerchlab/crm-cleanup@sha-44376c7817e7` (matching digest).
- This report and evidence.json are committed as the direct child of `3266c6b6c8d830e5f05447d93e9a1e3011ccb056` and describe that same
  revision; the recorded receipt_ref is the post-publish dogfood run of the published package, not a
  harness fixture seal.

## What to inspect first
1. `runx verify --receipt dogfood_receipt.json --json` (valid=true, production).
2. `evidence.json` dogfood.output (real takeaways + field_updates + gated write_proposal).
3. Raw X.yaml / SKILL.md / verification.json at source revision `3266c6b6c8d830e5f05447d93e9a1e3011ccb056`.
