# CRM Cleanup Skill - Delivery Report

## Overview
`crm-cleanup` (automerchlab/crm-cleanup@2.1.2) turns a call transcript into CRM field updates that are actually
executed. It is a three-step graph:

1. **read** - `steps/read_records.mjs` replays the account's append-only event log and
   returns the projected record and stream version;
2. **reconcile** - `steps/reconcile.mjs` extracts what the transcript asserts and keeps
   only the fields whose value differs from the record that was just read;
3. **write** - `steps/append_event.mjs` executes the decision under compare-and-set on
   the version from step 1 and seals `before_version -> after_version`, `event_ref` and a
   sha256 `event_digest`.

The write step is guarded on the reconcile decision, so a no-op run executes nothing.

## What the prior review asked for, and where it is
| Prior finding | Where it is answered |
| --- | --- |
| "reads no real source" | The dogfood reads stream `acct-northwind` at version 1 and gets back `budget=$40k, next_step=wait for procurement, status=contacted` - records written by an *earlier run of the same published package*, not a bundled fixture. See `evidence_json.observations[source_read]`. |
| "drives no consumed effect ... inert proposal object" | The decision is executed: `crm_records:acct-northwind:2`, version 1 -> 2, digest `sha256:1e09e709a51c36cf5f03f5662070ac5f1de665dd2391023f21445172e3be87dd`. See `observations[executed_write_result]`. |
| "local events.log labelled as a governed data-store append" | No such claim is made anywhere. `observations[transport_disclosure]` states plainly that this is a bounded bundled event log, why the native `data.*` tools cannot be used from a registry-installed package, and exactly which guarantees the transport does implement. |
| "add assertions proving the no-op path performs no write" | The `noop_no_write` harness case asserts `changed: false` and `field_updates: {}`; the second dogfood run's receipt contains **no `write_updates` step output at all** and leaves the stream at version 2. |
| "harness_cases do not match the X.yaml case names" | `evidence_json.dogfood.harness_cases` lists exactly `reconciled_write_sealed (sealed), noop_no_write (sealed), invalid_schema_refused (failed)`, the same names as `X.yaml harness.cases`. |
| "base64 node -e eval blob hiding the work" | Every step runs a checked-in script under `skills/crm-cleanup/steps/`; there is no inline code (runx itself rejects inline `-e` sources under strict workspace policy). |

## Dogfood, in order
1. **seed** - `Onboarding note. Their budget is $40k. The status is contacted. The next step is wait for procurement.` on an empty stream -> version 0 -> 1.
2. **reconcile** (the delivered receipt) - `Great call with Northwind. They have got around $75k to spend this quarter, so I am moving them to qualified. I will send over a proposal by Friday.` -> read version 1 record `budget=$40k, next_step=wait for procurement, status=contacted`,
   decide `budget=$75k, next_step=send over a proposal by Friday, status=qualified`, commit version 1 -> 2 as `crm_records:acct-northwind:2`.
3. **no-op** - `Quick sync, everything is on track, nothing to update right now.` -> `changed=false`, no write step, stream stays at version 2.

`runx verify` on the delivered receipt: **valid=true, signature_mode=production**.

## Maintainer-facing limits
- Extraction is deterministic pattern reading of English business transcripts: no coreference,
  no multi-account transcripts, no currency conversion. A field it cannot read is reported in
  `needs_review`, never invented.
- The bundled transport is local and single-writer. Its CAS guard protects against a stale
  decision overwriting a newer record, but it is not a distributed store.
- The record projection is last-write-wins per field; it does not model field-level history.

## Delivery references (single source revision)
- **package**: `automerchlab/crm-cleanup@2.1.2` - https://runx.ai/x/automerchlab/crm-cleanup@2.1.2
- **PR**: https://github.com/runxhq/runx/pull/264
- **source_url**: https://github.com/automerchlab/runx/tree/ed7d71d9c8168e0e85412cce04754947174dd492
- **raw X.yaml**: https://raw.githubusercontent.com/automerchlab/runx/ed7d71d9c8168e0e85412cce04754947174dd492/skills/crm-cleanup/X.yaml
- **raw SKILL.md**: https://raw.githubusercontent.com/automerchlab/runx/ed7d71d9c8168e0e85412cce04754947174dd492/skills/crm-cleanup/SKILL.md
- **verification.json**: https://raw.githubusercontent.com/automerchlab/runx/ed7d71d9c8168e0e85412cce04754947174dd492/verification.json
- **receipt_ref**: `runx:receipt:sha256:8f5a6c21275c90ae0b337a845cc7b1c6c6d52f5bd9dae548fd950a092429d3d6`
- **runx version**: runx-cli 0.8.2
