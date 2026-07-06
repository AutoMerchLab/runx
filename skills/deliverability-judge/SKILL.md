---
name: deliverability-judge
version: "0.1.0"
description: Fuse sealed provider evidence against operator policy thresholds into a read-only deliverability verdict and recommendation; refuse and escalate on contradictory or partial signals.
source:
  type: cli-tool
  command: node
  args:
    - run.mjs
runx:
  tags:
    - deliverability
    - email
    - judgment
    - read-only
links:
  source: https://github.com/automerchlab/runx/tree/main/skills/deliverability-judge
---

## What this skill does

`deliverability-judge` reads sealed provider evidence â€” postmaster reputation,
bounce rate, complaint rate, and an inbox placement probe â€” and fuses it against
operator policy thresholds to produce a single deliverability `verdict` and, when
warranted, a read-only `recommendation` (continue, throttle, or pause).

It sits upstream of a send gate. `send-as` gates a send by approval and a
provider delivers once approved, but neither judges whether the sending posture
is healthy enough to send at all. This skill makes that judgment and seals it as
a read-only recommendation a human or a downstream deliverability lane reads.

The skill is read-only (SHAPE-A): it mints no authority, holds no state, emits no
Effect, and cannot trigger a `send-as` run or a live throttle. When the T5
deliverability family ships a live throttle, that throttle is a separate governed
run an operator dispatches by naming; this judge never auto-executes it.

## Why this is a judgment, not a single-threshold tool

A single-threshold check is a tool. The judgment here is fusing signals that can
disagree and refusing to call a verdict when they contradict:

- When every signal is sealed, within policy, and consistent â†’ `verdict.healthy`
  with `recommendation.action = continue`.
- When signals agree on degradation (e.g. low reputation and high bounce both
  breach policy) â†’ a `degraded`/`critical` verdict with `throttle`/`pause`.
- When signals contradict (e.g. reputation reads healthy but bounce breaches
  policy) â†’ refuse to fuse, emit no recommendation, and escalate to a human,
  naming the contradicting signals. The refusal still seals.
- When the signal set is partial or unsealed â†’ refuse a verdict and name the
  missing signals. The skill never invents a signal it cannot find in evidence.

## Typed inputs

- `evidence` (object, required) â€” four sealed signal objects, each with `source`
  (string) and `timestamp` (ISO 8601):
  - `postmaster_report.reputation_score` (number)
  - `bounce_metrics.bounce_pct` (number, 0â€“100)
  - `complaint_metrics.complaint_pct` (number, 0â€“100)
  - `placement_probe.inbox_pct` (number, 0â€“100)
- `policy` (object, required) â€” `min_reputation_score`, `max_bounce_pct`,
  `max_complaint_pct`.
- `output_dir` (string, optional) â€” directory inside the skill dir where
  `evidence.json` and `report.md` are written.

## Typed output

`deliverability.judge.result.v1`:

- `verdict{ state, confidence_window, reason }` â€” always present. `state` is one
  of `healthy | degraded | critical | refused`.
- `recommendation{ action, signal_bindings, evidence_hash }` â€” present only when
  every signal is sealed and non-contradictory; otherwise `null`.
- `escalation{ kind, ... , route }` â€” present when the verdict is refused;
  `kind` is `contradictory_signals` or `missing_signals`, routed to a
  `human_reviewer`.
- `signals`, `contradictions`, `missing_signals`, and a `validation` block
  asserting read-only, no-state, no-invented-signals.

There is no `operational_proposal.v1` envelope and no `AttenuationRequest`: this
is a read-only verdict, not a money or effect handoff.

## Install, run, and verify

```bash
# install
runx add automerchlab/deliverability-judge@0.1.0

# run against sealed evidence (JSON with evidence{...} and policy{...})
runx skill automerchlab/deliverability-judge@0.1.0 --json < evidence-and-policy.json

# verify the sealed receipt
runx verify --receipt receipt.json --json
```

## Harness cases

- `sealed_healthy_signals_continue` â€” healthy reputation, low bounce, low
  complaint, passing placement probe fuse into `verdict.healthy` +
  `recommendation.action continue`; seals.
- `contradictory_signals_escalate` â€” reputation is healthy but bounce breaches
  policy; no recommendation is emitted and the refusal still seals.

## Edge cases and stop conditions

- Missing or unsealed signal â†’ refuse, escalate, name the missing signal.
- A signal object lacking `source` or `timestamp` is treated as unsealed.
- Asked to execute a throttle/pause, persist state, mint authority, or reach a
  money rail â†’ out of scope; the skill only emits a read-only recommendation.
- Never invents a signal absent from the evidence.
