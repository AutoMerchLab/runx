---
name: postmortem-maker
description: >-
  Reads a real incident thread at run time (a live web-fetch of a GitHub issue
  and its comments), reconstructs a cited timeline and a root cause it refuses
  to invent, and — only when the cause is confirmed — executes the publication
  through a bundled sealed outbox transport under compare-and-set, then reads
  the delivered message back and re-digests it. Conflicting or hedged evidence
  yields unknowns and publishes nothing, and the run proves the absence.
runx.category: ops
---

# postmortem-maker

Turn an incident thread into a postmortem that cites its own evidence, and
publish it only when the thread actually settles the cause.

The whole loop — read a real source, reconstruct, decide, deliver, read the
delivery back — happens in one sealed run, so the receipt shows what was read,
what was concluded, and what was (or was not) delivered.

## What it does

1. **Read** the incident from a real source at run time. `incident_source:
   {kind: "github_issue", ref: "https://api.github.com/repos/nltk/nltk/issues/3733"}`
   fetches the issue and its
   comments over HTTPS; every event keeps its upstream id, author, timestamp,
   and URL so later claims can cite it. `{kind: "inline", thread: {...}}` replays
   a bundled thread instead, which is what the harness cases use so they stay
   deterministic and egress-free.
2. **Reconstruct** the timeline. Each entry is a statement someone actually made
   — impact, cause claim, mitigation, or a timed note — carrying the event id,
   author, URL, and the quoted line it came from. Markdown headings and stray
   fragments are read for signal but never quoted as timeline entries.
3. **Decide the root cause without inventing one.** A cause is `confirmed` only
   when exactly one candidate is named in an unhedged causal statement.
   Restatements of the same cause across comments are merged by token overlap;
   competing candidates, or a thread where every cause statement is hedged
   ("might be", "I suspect", "not sure"), leave the cause `unconfirmed`, list
   the open questions in `unknowns[]`, and add the action item that has to close
   before anyone publishes.
4. **Deliver, when authorized.** The send plan binds principal, provider,
   channel, audience, content digest, consent basis, and the approval gate. When
   the gate opens, the provider adapter executes the send: the postmortem is
   appended to the outbox stream under compare-and-set, gets a provider message
   id, and is durable on disk. When the gate stays shut, this step never runs.
5. **Read the delivery back.** The last step never trusts the delivery report.
   It re-opens the outbox itself, finds the message, re-digests the *stored*
   bytes, and compares against the digest the plan authorized. On the withheld
   path it asserts the opposite: no send plan authorized, no provider act, no
   delivery for this incident, and an outbox version unchanged from what was
   read before the run decided.

## What the transport is, stated plainly

This package ships its own provider adapter: an **append-only outbox log bundled
with the skill**. It is not a hosted provider and not the runx data-store.

Two facts make that the honest choice rather than a shortcut. The canonical
`runx/send-as` skill describes itself as a planning and authority layer that
"never delivers", and refers actual delivery to a provider adapter. And the
runtime's native `data.*` tools are not in the execution closure of a package
installed from the registry, so a published skill cannot call them.

So the send plan here is send-as **shaped** — same authority model: who speaks,
to whom, through which channel, over which content digest, under which consent
basis and approval gate — and the delivery is performed by this package's own
adapter, which really does what it claims:

- **compare-and-set** — the append is refused unless the outbox is still at the
  version read before the decision, so a concurrent publisher cannot be clobbered.
- **idempotency** — republishing the same postmortem returns the original
  delivery instead of sending twice; the same key with different content is refused.
- **durability and readback** — the message is on disk after the run, which is
  what lets the verify step, and any later run, read it back independently.

Nothing in the output claims a hosted provider delivered anything.

## Inputs

| input | type | required | notes |
|-------|------|----------|-------|
| `incident_source` | json | yes | `{kind: "github_issue", ref}` for a live HTTPS read, or `{kind: "inline", thread}` for a replayed thread. |
| `publish_target` | json | yes | `{data_source_ref, channel, aggregate_id}` plus optional `principal`, `audience`, `classification`, `visibility`. |
| `postmortem_policy` | json | no | `require_confirmed_root_cause` (default `true`) withholds publication until one cause is confirmed. |

## Outputs

- `postmortem` — `title`, `source{ref, read_mode, events_read, source_digest}`,
  `timeline[]` (each with `statement`, `kind`, `confidence`, and
  an `evidence{event_id, author, at, url, quote}` citation), `root_cause{status,
  statement, citations[], corroborated_by_mitigation}`, `mitigation`,
  `unknowns[]`, `action_items[]`, `status`.
- `publishable`, `root_cause_status`, `timeline_count`, `content_digest`,
  `idempotency_key`, `expected_version`.
- `send_plan` — the send-as shaped authority record, `status` `authorized` or
  `withheld` with the reason.
- `delivery_result` — on the published path: `status: delivered`, `provider`,
  `operation: send`, `message_id`, `message_ref`, `content_digest`,
  `before_version` → `after_version`, `delivered_at`, `replayed`.
- `readback` — on the published path `delivered: true` with `digest_match: true`
  and the stored `message_id`; on the withheld path `delivered: false` with
  `send_plan_created: false`, `provider_act_performed: false`,
  `delivery_exists: false`, and `outbox_unchanged: true`.

## Harness

- `consistent-incident-published` — one change is named in an unhedged causal
  statement and the rollback corroborates it: the postmortem is authorized, the
  adapter executes the send (`before_version` 0 → `after_version` 1), and the
  readback re-digests the stored message (`digest_match: true`).
- `conflicting-evidence-withheld` — two changes compete and every statement is
  hedged: `unknowns[]` records the open questions, nothing is delivered, and the
  readback asserts the absence (`delivery_exists: false`, `outbox_unchanged: true`).
- `empty-thread-refused` — a thread with nothing readable in it: the run refuses
  rather than emitting an empty postmortem.

## Install, run, verify

```bash
# Install from the registry
runx add automerchlab/postmortem-maker@2.0.1 --registry https://api.runx.ai

# Run over a real incident thread (fetched over HTTPS at run time)
runx skill automerchlab/postmortem-maker@2.0.1 --registry https://api.runx.ai --json \
  --input-json incident_source='{"kind":"github_issue","ref":"https://api.github.com/repos/nltk/nltk/issues/3733"}' \
  --input-json publish_target='{"data_source_ref":"local://runx-postmortems/dogfood","channel":"incident-review","aggregate_id":"nltk-3733","principal":"incident-review-bot","audience":"incident-review"}' \
  -R ./receipts

# Verify the sealed receipt (production signature). The receipt store also holds
# an index.json, which is not a receipt — pick the newest sealed receipt file:
runx verify --receipt "$(ls -t ./receipts/*.json | grep -v index.json | head -1)" --json
# expects valid=true, signature_mode=production
```

Run the same command twice: the second run returns the original delivery with
`replayed: true` and leaves the outbox version where it was — the message from
the first run is still there, and the second run reads it back.

## Limitations

- Analysis is deterministic text analysis over the thread, not an LLM: a cause
  stated only in an image, a linked dashboard, or a private channel is not
  visible to it, and the run will say the cause is unconfirmed rather than guess.
- Cause candidates are merged by significant-token overlap. Two genuinely
  different causes that share a word can merge; two phrasings of one cause that
  share none can stay split and hold publication back. It fails toward
  `unconfirmed`, which withholds publication rather than publishing a guess.
- The outbox transport is bundled and local, as described above. Point
  `publish_target.data_source_ref` at the review surface you actually want in
  your own deployment, or wrap this skill with a provider adapter of your own.
- The GitHub read is unauthenticated, so it is subject to public rate limits and
  cannot read private incident threads.
