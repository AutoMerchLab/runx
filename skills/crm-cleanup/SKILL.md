---
name: crm-cleanup
description: Reads an account's current CRM records from an append-only event log, reconciles a call transcript against them, and executes only the fields that actually changed as a compare-and-set append that seals a before/after write result.
---

# CRM Cleanup

`crm-cleanup` keeps pipeline data from rotting after a call. It is a three-step
graph, not a text extractor bolted onto a proposal object:

1. **Read** — `steps/read_records.mjs` replays the account's append-only event
   log and returns the projected record plus the stream version.
2. **Reconcile** — `steps/reconcile.mjs` reads the transcript, extracts the value
   each `crm_schema` field is asserted to have, and keeps **only the fields whose
   asserted value differs from what the log already holds**.
3. **Write** — `steps/append_event.mjs` executes the decision under
   compare-and-set on the version read in step 1, sealing `before_version` →
   `after_version`, `event_ref`, and a sha256 `event_digest`.

The write step is guarded on the reconcile decision. When nothing differs, step 3
never runs: no event is appended and the stream version is unchanged.

## What the transport is, stated plainly

Steps 1 and 3 are a **bounded append-only event log bundled with this skill**.
They are not the runx data-store, and nothing here is a governed data-store
append.

That is a deliberate choice, not a shortcut. runx ships native `data.*` tools
(`data.read_projection`, `data.append_event`, `data.source`) and this skill was
first built on them: the harness passed and a local run sealed a real
`sqlite-event-store` append with `before_version 0 → after_version 1`. But those
tools are not in the execution closure of a package installed from the registry.
Running the published package fails before the first step with:

```
skill 'read_records' failed: Imported tool 'data.read_projection'
was not found in configured tool catalogs
```

`runx/data-store` is also not resolvable from the hosted registry, so declaring
it through `context_skills` does not help either. A skill that must run from
`runx skill automerchlab/crm-cleanup@2.1.2` therefore cannot call them, and this
package carries its own transport instead — with the same guarantees written
out explicitly:

- **compare-and-set** — the append is refused unless the stream is still at the
  version step 1 read, so a concurrent writer is never clobbered;
- **idempotency** — replaying one key with the same content returns the original
  commit; the same key with different content is refused;
- **sealed before/after** — `before_version` → `after_version`, `event_ref`, and
  a sha256 digest over the canonical event bytes, all recorded in the receipt.

The log lives at `.crm-store/<addressed-id>.jsonl` under the run's working
directory, one JSON line per event. The records step 1 returns are whatever
earlier appends left there; no record fixture is bundled and read off disk.

## Typed contract

**Inputs**

| input | type | meaning |
| --- | --- | --- |
| `source_handle` | json | `data_source_ref`, `resource`, `aggregate_id` — addresses one account's stream |
| `transcript` | string | raw call or meeting transcript |
| `crm_schema` | string | JSON object whose `fields` array is the write allowlist |

**Outputs** (`result_from: reconcile, write_updates`)

| output | type | meaning |
| --- | --- | --- |
| `takeaways` | array | reconciliation notes, including fields that already matched the record |
| `field_updates` | object | keyed by `crm_schema` field; only fields that actually change |
| `needs_review` | array | lines that assert a change to a schema field but yield no readable value |
| `changed` | boolean | whether the write step is allowed to run |
| `expected_version` | number | the version read in step 1, used as the CAS guard |
| `idempotency_key` | string | stable retry key over aggregate + version + decision |
| `before` | object | the field values held before the run |
| `write_result` | object | the executed append: `status`, `before_version`, `after_version`, `event_ref`, `event_digest`, `store_path`, `before`, `after` |

Every `field_updates` key comes from `crm_schema.fields`; a field outside that
allowlist is never written. No secret material is read or emitted.

## How extraction works

Extraction is deterministic — regex heuristics plus small synonym vocabularies,
never an LLM — so every run seals reproducibly. Each field is read according to
a type inferred from its name (money, enum, action, or generic), and each type
understands several natural phrasings rather than one `<field> is <value>`
template: "they have got around $75k to spend", "I am moving them to qualified",
and "I will send over a proposal by Friday" all resolve. A field is only bound
from an unnamed sentence when it is the sole field of its type, so two same-type
fields never collapse onto one value.

When a sentence clearly refers to a schema field and carries a change cue but no
value can be pulled out, the line is recorded in `needs_review` instead of being
silently dropped, so a human can triage it. Negation cues ("nothing to update",
"on track") suppress review noise.

Scope, stated honestly: this is pattern-based reading of English business
transcripts. It does not do coreference, multi-account transcripts, or currency
conversion, and it will not invent a value it cannot read — that is what
`needs_review` is for.

## Harness

Three cases run with `runx harness ./skills/crm-cleanup`:

| case | proves |
| --- | --- |
| `reconciled_write_sealed` | a clear transcript reconciled against stored records yields `field_updates` **and** an executed append (asserted: `changed: true`, `budget: $75k`, `status: qualified`, `status: committed`, `operation: append_event`, `before_version: 0`, `after_version: 1`, `replayed: false`) |
| `noop_no_write` | the no-op path executes nothing (asserted: `changed: false`, `field_updates: {}`, so the guarded write step never runs and the stream version is untouched) |
| `invalid_schema_refused` | an empty `crm_schema` is refused rather than silently writing nothing |

The harness needs no network. `fixtures/seed-crm-records.json` documents the
record snapshot used to prime a stream before a dogfood run; it is a seed input,
never a source the skill reads at run time.

## Install, run, verify

```bash
runx add automerchlab/crm-cleanup@2.1.2 --registry https://api.runx.ai

# 1. establish the account's current records (first append on an empty stream)
runx skill automerchlab/crm-cleanup@2.1.2 --registry https://api.runx.ai --json \
  --input-json source_handle='{"data_source_ref":"local://runx-crm/dogfood","resource":"crm_records","aggregate_id":"acct-northwind"}' \
  -i transcript='Onboarding note. Their budget is $40k. The status is contacted. The next step is wait for procurement.' \
  -i crm_schema='{"fields":["budget","status","next_step"]}'

# 2. reconcile a later call against those records and execute the updates
runx skill automerchlab/crm-cleanup@2.1.2 --registry https://api.runx.ai --json \
  --input-json source_handle='{"data_source_ref":"local://runx-crm/dogfood","resource":"crm_records","aggregate_id":"acct-northwind"}' \
  -i transcript='Great call. They have got around $75k to spend this quarter, so I am moving them to qualified. I will send over a proposal by Friday.' \
  -i crm_schema='{"fields":["budget","status","next_step"]}' \
  -R ./receipts

runx verify --receipt "$(ls ./receipts/sha256:*.json | head -1)" --json
```

Run 2 reports `before` from run 1's record, writes only the fields that changed,
and seals `before_version 1 → after_version 2`. Repeating run 2 verbatim is
refused by the idempotency guard rather than double-writing.
