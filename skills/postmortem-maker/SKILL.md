---
name: postmortem-maker
description: Turns incident fragments (timeline events, alerts, deploy events, chat notes, policy) into a traceable postmortem packet that separates known facts from hypotheses, keeps unresolved questions in unknowns, and emits a gated publish proposal without posting or assigning anything.
---

# Postmortem Maker

The `postmortem-maker` skill folds raw incident fragments — timeline events,
alerts, deploy events, chat notes, and a postmortem policy — into a single
traceable postmortem packet. Its core rule: **it never pretends unknowns are
facts.** Every timeline entry and every root-cause claim cites the exact input
item it came from, hedged speculation stays a hypothesis, and anything the
evidence cannot settle is emitted under `unknowns` instead of being written into
the narrative.

It is read-only with respect to the world: it posts nothing, assigns no work,
and its `publish_proposal` is a gated object (`requires_approval: true`)
consumed by a downstream send-as or doc-publisher executor.

## How folding works

Processing is deterministic (time correlation plus hedge-cue classification
over regex, not an LLM), so runs are reproducible and seal identically in the
harness:

- **Unified timeline.** Timeline events, alerts, and deploys are merged and
  chronologically sorted; every row carries an `evidence` citation such as
  `deploy_events[0]` or `alerts[1]`. Undated fragments keep their input order at
  the end rather than being given an invented position.
- **Impact.** Taken from an explicit `impact` field on a timeline event, or from
  an impact-bearing phrase (error rate, latency, affected users, downtime) in an
  event or alert. When nothing quantifies impact, `impact.status` is `unknown`
  and a corresponding question is added to `unknowns`.
- **Facts vs hypotheses.** A chat note asserting a cause declaratively
  ("the v2.4.1 deploy introduced a null pointer") is a confirming fact. A note
  with hedge cues ("might be…", "I suspect…", "not sure", a question mark) is a
  hypothesis and can never confirm a root cause by itself.
- **Root cause.** Deploys landing within `max_correlation_window_min` (default
  30) before the first alert are candidates. One candidate plus a declarative
  confirming note → `confirmed`. One candidate with no competing speculation →
  `probable`, with a confirmation question in `unknowns`. Multiple candidates or
  notes blaming different services → `unknown`, and each candidate/hypothesis
  becomes an `unknowns` entry to rule in or out. No candidate at all → `unknown`.
- **Action items.** Each action item names a target lane (`improve-skill`,
  `policy-author`, `ops`) and cites its grounding evidence. The skill only
  proposes; a downstream driver issues any actual work.
- **Publish gate.** A `publish_proposal` is drafted only when the policy's bar is
  met (by default a `confirmed` root cause AND known impact). Otherwise
  `publish_proposal` is `null` — insufficient or conflicting evidence never
  produces a publish proposal.

## Inputs

- `incident_timeline` (array, required): Incident events, each ideally
  `{ at, event, impact? }`. `at` accepts ISO 8601 or `HH:MM`.
- `alerts` (array, required): Fired alerts, each ideally `{ at, name, severity }`.
- `deploy_events` (array, required): Deploys, each ideally
  `{ at, service, version }`.
- `chat_notes` (array, optional): Responder notes, each `{ at, author, text }`
  or a plain string.
- `postmortem_policy` (object, optional): Governs the packet:
  `require_confirmed_root_cause` (default `true`),
  `max_correlation_window_min` (default `30`),
  `publish_target` (default `"incident-review"`),
  `visibility` (default `"internal"`).

An incident where `incident_timeline`, `alerts`, and `deploy_events` are all
empty is refused: there is no evidence to fold, and any postmortem would be
invented.

## Outputs

- `postmortem` (object): `{ summary, timeline, impact, root_cause, status }`.
  - `timeline`: merged chronological rows, each
    `{ at, kind: event|alert|deploy, description, evidence }`.
  - `impact`: `{ summary, status: known|unknown, evidence? }`.
  - `root_cause`: `{ statement, status: confirmed|probable|unknown, evidence[] }`.
  - `status`: `complete` (confirmed cause + known impact), `draft`, or
    `needs_review` (cause unknown).
- `unknowns` (array): Open questions the evidence cannot settle, each
  `{ question, reason, evidence? }`. Unresolved facts live here, never in the
  narrative.
- `action_items` (array): `{ title, owner_lane, priority, evidence[] }` — every
  item names its target lane and cites grounding evidence.
- `publish_proposal` (object or null): When the policy bar is met,
  `{ action, target, visibility, title, requires_approval: true, note, grounded_in }`.
  Always gated; this skill posts nothing and assigns no live tasks.

## Example — consistent evidence (sealed)

**Input (abridged):** one critical alert at 10:03, a `checkout-api@v2.4.1`
deploy at 09:58, a timeline event with quantified impact, and a declarative
chat note: "The 09:58 checkout-api v2.4.1 deploy introduced a null pointer…".

**Output (abridged):**
```json
{
  "postmortem": {
    "root_cause": {
      "statement": "Deploy of checkout-api@v2.4.1 at 09:58 5 min before the first alert",
      "status": "confirmed",
      "evidence": ["deploy_events[0]", "alerts (first at minute 603)", "chat_notes[0]"]
    },
    "impact": { "summary": "~1200 users saw failed checkouts over 18 minutes", "status": "known", "evidence": "incident_timeline[0]" },
    "status": "complete"
  },
  "unknowns": [],
  "action_items": [
    { "title": "Add an automated rollback / deploy guard for checkout-api", "owner_lane": "improve-skill", "priority": "high", "evidence": ["deploy_events[0]", "alerts (first at minute 603)"] }
  ],
  "publish_proposal": { "action": "publish_postmortem", "requires_approval": true, "target": "incident-review", "visibility": "internal" }
}
```

## Example — conflicting evidence (uncertain, no proposal)

**Input (abridged):** two deploys (`api-gateway@v1.2.0`, `search@v3.1.0`) both
inside the correlation window, and two hedged notes blaming different services
("Might be the api-gateway deploy?", "I suspect the search change, not sure").

**Output (abridged):**
```json
{
  "postmortem": { "root_cause": { "statement": "Undetermined: conflicting candidates", "status": "unknown", "evidence": [] }, "status": "needs_review" },
  "unknowns": [
    { "question": "Rule in/out candidate: Deploy of api-gateway@v1.2.0 at 13:50 16 min before the first alert", "reason": "Multiple deploys correlate with the alert window" },
    { "question": "Rule in/out candidate: Deploy of search@v3.1.0 at 13:55 11 min before the first alert", "reason": "Multiple deploys correlate with the alert window" },
    { "question": "Hypothesis to verify: Might be the api-gateway v1.2.0 deploy?", "reason": "Hedged speculation in chat, not corroborated" },
    { "question": "Hypothesis to verify: I suspect the search v3.1.0 change instead, not sure.", "reason": "Hedged speculation in chat, not corroborated" }
  ],
  "publish_proposal": null
}
```

## Install, run, verify

```bash
# Install from the registry
runx add automerchlab/postmortem-maker@1.0.0 --registry https://api.runx.ai

# Run against a real incident (inputs as JSON)
runx skill automerchlab/postmortem-maker@1.0.0 --registry https://api.runx.ai --json \
  --input-json incident_timeline='[{"at":"10:02","event":"Checkout error rate spiked to 40%","impact":"~1200 users saw failed checkouts over 18 minutes"}]' \
  --input-json alerts='[{"at":"10:03","name":"CheckoutErrorRateHigh","severity":"critical"}]' \
  --input-json deploy_events='[{"at":"09:58","service":"checkout-api","version":"v2.4.1"}]' \
  --input-json chat_notes='[{"at":"10:10","author":"oncall","text":"The 09:58 checkout-api v2.4.1 deploy introduced a null pointer in the payment path; rolling back now."}]' \
  --input-json postmortem_policy='{"require_confirmed_root_cause":true,"max_correlation_window_min":30}' \
  -R ./receipts

# Verify the sealed receipt
runx verify --receipt ./receipts/sha256-<id>.json --json
# expects valid=true, signature_mode=production
```

## Limitations

- Correlation is temporal, not causal proof: a deploy inside the window plus a
  declarative note yields `confirmed`; without the note the best verdict is
  `probable`. By design the skill under-claims rather than over-claims.
- Hedge/cause detection is a fixed English cue vocabulary; a note phrased
  outside it is treated as ordinary chatter (never silently promoted to a fact).
- Undated fragments cannot participate in correlation; they stay on the
  timeline tail and the root cause falls back to `unknown` when timing is
  missing.
- The skill never posts, assigns, or publishes anything. `publish_proposal` is a
  proposal only, and it is `null` whenever the evidence bar is not met.
