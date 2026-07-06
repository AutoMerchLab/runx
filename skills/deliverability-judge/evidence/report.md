# Deliverability Judge Skill - Delivery Report (Frantic #65)

## Package
- **Skill** `deliverability-judge` | **Owner** `automerchlab` | **Version** `0.1.0`
- **Registry ref** `automerchlab/deliverability-judge@0.1.0` | **digest** `55eb8cbbad1254f409e755a48898ad673071ec891c269c8de5a6f05a58e2ab35` | **profile_digest** `6ccf3590a5edb5adfce61f80f1b7b549d2ad6f11e039b54b57e91d430e891ba7`
- **public_url** https://runx.ai/x/automerchlab/deliverability-judge@0.1.0
- **pr_url** https://github.com/runxhq/runx/pull/271
- **source_url** https://github.com/automerchlab/runx/tree/cda4e6d34f2c2153d38dbfcaec80bb5f14d703e9
- **raw X.yaml** https://raw.githubusercontent.com/automerchlab/runx/cda4e6d34f2c2153d38dbfcaec80bb5f14d703e9/skills/deliverability-judge/X.yaml
- **raw SKILL.md** https://raw.githubusercontent.com/automerchlab/runx/cda4e6d34f2c2153d38dbfcaec80bb5f14d703e9/skills/deliverability-judge/SKILL.md

## runx CLI
`runx --version` -> **runx-cli 0.6.14** (>= 0.6.13). Used for publish, install, dogfood, verify.

## What the skill does (maintainer-facing value)
`deliverability-judge` sits upstream of a send gate. It fuses four sealed provider signals
(postmaster reputation, bounce rate, complaint rate, inbox placement probe) against operator
policy thresholds and produces a single read-only verdict plus a recommendation (continue /
throttle / pause). The judgment is the fusion: signals that agree on degradation produce a
throttle/pause; signals that **contradict** (healthy reputation but breaching bounce/complaint)
are refused and escalated to a human, never resolved into a false verdict; a partial or unsealed
signal set is refused and the missing signals are named; no signal is ever invented. It is
read-only (SHAPE-A): mints no authority, holds no state, emits no Effect.

## Install (clean)
`runx add automerchlab/deliverability-judge@0.1.0 --registry https://api.runx.ai` -> source=remote, status=success.

## Harness
`runx harness ./skills/deliverability-judge` -> **2/2 PASSED, 0 assertion errors** (WSL Linux local).
Cases: **sealed_healthy_signals_continue** (sealed - four sealed signals fuse into verdict.healthy +
recommendation.action continue), **contradictory_signals_escalate** (failure/sealed - reputation 92 >= 80 but
bounce 8.5% > 5% contradict; no recommendation, refusal sealed, escalated to human_reviewer).
The runx CLI exposes no client-side hosted-harness call; the hosted registry harness runs the same two
inline X.yaml cases server-side at review time.

## Dogfood (post-publish, real)
- Command: `runx skill automerchlab/deliverability-judge@0.1.0 --registry https://api.runx.ai --json --input-json evidence='{"postmaster_report": {"source": "postmaster.google.com", "timestamp": "2026-06-25T12:00:00Z", "reputation_score": 95, "domain": "example.com"}, "bounce_metrics": {"source": "esp-bounce-monitor", "timestamp": "2026-06-25T12:00:00Z", "bounce_pct": 1.2}, "complaint_metrics": {"source": "esp-feedback-loop", "timestamp": "2026-06-25T12:00:00Z", "complaint_pct": 0.05}, "placement_probe": {"source": "seedlist-placement-probe", "timestamp": "2026-06-25T12:00:00Z", "inbox_pct": 97.5}}' --input-json policy='{"min_reputation_score": 80, "max_bounce_pct": 5, "max_complaint_pct": 0.3}' -R ./receipts`
- Output: verdict **healthy** (confidence 7d); recommendation **continue**; evidence_hash `sha256:384bd388046cd00a4c2ef369387390252dd367ee08d0b384324bc343f0d21f9b`.
- Receipt: `runx:receipt:sha256:89f6a06de8e95c92e650d77b1e1af38a142af8efcd1f285ea69c717335793669`
- `runx verify --receipt dogfood_receipt.json --json` -> **valid: true, signature_mode: production, signature: valid, digest: valid**.

## Provenance
All bound artifact URLs pin to a single PR head commit on `automerchlab/runx` `deliverability-judge`
(PR against runxhq/runx). The skill files are byte-identical to published `automerchlab/deliverability-judge@0.1.0` (digest 55eb8cbbad1254f409e755a48898ad673071ec891c269c8de5a6f05a58e2ab35).
The recorded receipt_ref is the post-publish dogfood run of the published package, not a harness fixture seal.

## What to inspect first
1. `runx verify --receipt dogfood_receipt.json --json` (valid=true, production).
2. `evidence.json` dogfood.output (verdict + signals + recommendation with evidence_hash) and observations.
3. Raw X.yaml / SKILL.md at the PR head commit; confirm the two harness case names.
