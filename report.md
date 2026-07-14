# Postmortem Maker Skill - Delivery Report (bounty #83)

## Overview
`postmortem-maker` turns incident fragments into a traceable postmortem **without
pretending unknowns are facts**. It reads incident timeline events, alerts, deploy
events, chat notes, and a postmortem policy; separates known facts from hypotheses;
and emits a postmortem packet with action items and a **gated** publish proposal.
It never posts or assigns work directly.

- Facts vs hypotheses: a declarative chat note ("the v1.9.3 deploy introduced ...")
  can confirm a cause; a hedged note ("might be...", "I suspect...") is a hypothesis
  and never confirms anything on its own.
- Every timeline entry and root-cause claim cites its input evidence
  (`incident_timeline[i]`, `alerts[i]`, `deploy_events[i]`, `chat_notes[i]`).
- Conflicting or insufficient evidence yields `unknowns` and **no** publish proposal;
  an empty incident (no timeline/alerts/deploys) is refused outright.
- `publish_proposal` is a gated object (`requires_approval: true`) consumed by
  send-as or the doc-publisher executor.

## Package
- **Skill**: `postmortem-maker` | **Owner**: `automerchlab` | **Version**: `1.0.0`
- **Registry ref**: `automerchlab/postmortem-maker@1.0.0` (runx registry read automerchlab/postmortem-maker@1.0.0 --json resolves metadata + digests)
- **public_url**: https://runx.ai/x/automerchlab/postmortem-maker@1.0.0
- **pr_url**: https://github.com/runxhq/runx/pull/320
- **source_url**: https://github.com/automerchlab/runx/tree/b0ad3f4545a528a083317b92524eabf7fbdf9830
- **raw X.yaml**: https://raw.githubusercontent.com/automerchlab/runx/b0ad3f4545a528a083317b92524eabf7fbdf9830/skills/postmortem-maker/X.yaml
- **raw SKILL.md**: https://raw.githubusercontent.com/automerchlab/runx/b0ad3f4545a528a083317b92524eabf7fbdf9830/skills/postmortem-maker/SKILL.md
- **verification_json**: https://raw.githubusercontent.com/automerchlab/runx/b0ad3f4545a528a083317b92524eabf7fbdf9830/verification.json

## runx CLI
- `runx --version` -> **runx-cli 0.6.16** (>= 0.6.14 floor). Used for publish, install, dogfood, and verify.

## Publish & install
- Publish: `runx login --provider github --for publish`, then
  `runx registry publish ./skills/postmortem-maker/SKILL.md --registry https://api.runx.ai --version 1.0.0`.
- Clean install: `runx add automerchlab/postmortem-maker@1.0.0 --registry https://api.runx.ai` -> source=remote, status=installed
  (installed payload includes run.mjs, X.yaml, SKILL.md; digest sha256:4059df32bd27f27b37e6c3c6afbe6743a1049d8fd4d52daf0f572357b95a9fc2).

## Harness
- Local harness: `runx harness ./skills/postmortem-maker` -> **3/3 cases, 0 assertion errors** (WSL Linux local).
- Cases: consistent-incident-sealed (sealed), conflicting-evidence-uncertain (sealed), missing-inputs-refused (refused).
  - **consistent-incident-sealed** — one deploy 5 min before the alert + a declarative confirming note +
    quantified impact -> postmortem complete, action_items, gated publish_proposal.
  - **conflicting-evidence-uncertain** — two deploys in-window + two hedged notes blaming different
    services -> root_cause unknown, 4 unknowns (each candidate/hypothesis), **publish_proposal null**;
    seals deterministically.
  - **missing-inputs-refused** — no timeline/alerts/deploys -> refused ("any postmortem would be invented").
- Harness evidence is committed in the PR: `skills/postmortem-maker/harness/harness_out.json`
  and the sealed harness receipts under `skills/postmortem-maker/harness/receipts/`.

## Dogfood (post-publish, real, against the PUBLISHED package)
- Command: `runx skill automerchlab/postmortem-maker@1.0.0 --registry https://api.runx.ai --json --input-json incident_timeline='[{"at": "2026-07-14T08:50:00Z", "event": "Webhook delivery failures climbed to 35%", "impact": "312 merchant webhook endpoints missed order notifications for 22 minutes"}, {"at": "2026-07-14T09:12:00Z", "event": "Dispatcher rolled back to v1.9.2, failure rate recovered"}]' --input-json alerts='[{"at": "2026-07-14T08:52:00Z", "name": "WebhookDeliveryFailureRateHigh", "severity": "critical"}]' --input-json deploy_events='[{"at": "2026-07-14T08:41:00Z", "service": "webhook-dispatcher", "version": "v1.9.3"}]' --input-json chat_notes='[{"at": "2026-07-14T08:58:00Z", "author": "oncall", "text": "The v1.9.3 webhook-dispatcher deploy introduced a malformed signature header, receivers reject the payload; rolling back."}, {"at": "2026-07-14T09:13:00Z", "author": "oncall", "text": "Rollback done, deliveries recovering."}]' --input-json postmortem_policy='{"require_confirmed_root_cause": true, "max_correlation_window_min": 30, "publish_target": "incident-review", "visibility": "internal"}' -R ./receipts`
- The receipt's registry provenance proves the published package was run:
  registry_source=remote https://api.runx.ai, skill_id=automerchlab/postmortem-maker, version=1.0.0, trust_state=trusted, trust_tier=community.
- Output: root_cause **confirmed** (webhook-dispatcher@v1.9.3 deploy 11 min before the first alert,
  corroborated by the declarative oncall note), impact **known** (312 merchant endpoints, 22 minutes),
  postmortem status **complete**, 0 unknowns, 2 action items (each naming a target lane),
  and a **gated** publish proposal (requires_approval=true).
- Receipt: `runx:receipt:sha256:de99603f53042768c4250bc38a89f2eeee31e7f7754c6bca07f782c588d40f0a`
- `runx verify --receipt dogfood_receipt.json --json` -> **valid: true, signature_mode: production, signature: valid**.

## Provenance (single source revision)
- source_url, raw X.yaml, raw SKILL.md and verification.json all resolve at one source revision:
  commit `b0ad3f4545a528a083317b92524eabf7fbdf9830` on the `automerchlab/runx` `postmortem-maker` branch (the PR's head lineage).
- The committed skill files are the files that were published as `automerchlab/postmortem-maker@1.0.0` and the dogfood ran that
  published package from the remote registry — not a local path (receipt registry_provenance above).
- This report and evidence.json are committed as the direct child of `b0ad3f4545a528a083317b92524eabf7fbdf9830` and describe that same
  revision; the recorded receipt_ref is the post-publish dogfood run, not a harness fixture seal.

## How a new user installs, runs, verifies (no private context)
1. `runx add automerchlab/postmortem-maker@1.0.0 --registry https://api.runx.ai`
2. `runx skill automerchlab/postmortem-maker@1.0.0 --registry https://api.runx.ai --json --input-json incident_timeline='[{"at": "2026-07-14T08:50:00Z", "event": "Webhook delivery failures climbed to 35%", "impact": "312 merchant webhook endpoints missed order notifications for 22 minutes"}, {"at": "2026-07-14T09:12:00Z", "event": "Dispatcher rolled back to v1.9.2, failure rate recovered"}]' --input-json alerts='[{"at": "2026-07-14T08:52:00Z", "name": "WebhookDeliveryFailureRateHigh", "severity": "critical"}]' --input-json deploy_events='[{"at": "2026-07-14T08:41:00Z", "service": "webhook-dispatcher", "version": "v1.9.3"}]' --input-json chat_notes='[{"at": "2026-07-14T08:58:00Z", "author": "oncall", "text": "The v1.9.3 webhook-dispatcher deploy introduced a malformed signature header, receivers reject the payload; rolling back."}, {"at": "2026-07-14T09:13:00Z", "author": "oncall", "text": "Rollback done, deliveries recovering."}]' --input-json postmortem_policy='{"require_confirmed_root_cause": true, "max_correlation_window_min": 30, "publish_target": "incident-review", "visibility": "internal"}' -R ./receipts`
3. `runx verify --receipt dogfood_receipt.json --json (or the new receipt file your own run writes under ./receipts)` -> valid=true, signature_mode=production.

## What to inspect first
1. `runx verify --receipt dogfood_receipt.json --json` (valid=true, production).
2. `evidence.json` dogfood.output (postmortem with per-row evidence citations, unknowns, lane-named
   action_items, gated publish_proposal).
3. Raw X.yaml / SKILL.md / verification.json at source revision `b0ad3f4545a528a083317b92524eabf7fbdf9830`.
4. The conflicting-evidence-uncertain harness case: unknowns populated, proposal null.
