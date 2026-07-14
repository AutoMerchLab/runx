---
name: crm-cleanup
description: Extracts structured CRM field updates from a natural-language meeting transcript and flags actionable lines it cannot confidently parse for human review.
---

# CRM Cleanup

The `crm-cleanup` skill turns a messy meeting or call transcript into structured
CRM field updates. For each field declared in `crm_schema`, it reads the
transcript with a small set of deterministic heuristics and either extracts a
value or, when a line clearly refers to that field and asserts a change but no
value can be pulled out, records it under `needs_review` so a human can triage
it instead of the update being silently lost.

## How extraction works

Extraction is deterministic (regex heuristics plus small synonym vocabularies),
not an LLM, so runs are reproducible and seal identically in the harness. Each
field is read according to a type inferred from its name:

- **money** (e.g. `budget`, `amount`, `deal_size`): captures currency amounts in
  several forms — `$50k`, `$120,000`, `around $75k`, `1.2m` — and normalizes the
  suffix (`$50k`, `$1.2m`).
- **enum / status** (e.g. `status`, `stage`, `priority`): maps to a controlled
  status vocabulary (new, contacted, qualified, negotiation, proposal, won,
  lost, on hold, churned, …), and only when the term follows a transition cue
  (`moving them to qualified`, `mark as won`, `status is qualified`, `now
  negotiation`) so a temporal mention like "after the demo" is not misread as a
  status value.
- **action / next step** (e.g. `next_step`, `action`, `follow_up`): captures the
  action clause from phrasings such as `next step is …`, `follow up with …`,
  `I'll send …`, `let's schedule …`, `action item: …`.
- **generic** (any other field name): matches `set <field> to <value>`,
  `<field> is/=/: <value>`, and `<field> of <value>`.

Because the field type is inferred from the schema, the skill works over an
arbitrary `crm_schema`, not just the three example fields.

## Inputs

- `transcript` (string, required): The raw or lightly-edited text transcript from
  the meeting or call.
- `crm_schema` (string, required): A JSON string containing a `fields` array of
  allowed field names, e.g. `{"fields": ["budget", "status", "next_step"]}`. A
  missing or malformed schema (no `fields` array) is refused.

## Outputs

- `takeaways` (array of strings): One line per extracted field describing what was
  read from the transcript, e.g. `"Identified budget: $50k"`.
- `field_updates` (object): Field name → extracted value, keyed to `crm_schema`
  fields. Only fields with a confident value appear here.
- `needs_review` (array of objects): Lines that reference a schema field with an
  actionable cue but yielded no confident value. Each item is
  `{ field, line, reason }` so a human can resolve the ambiguity. This is what
  keeps an unparseable-but-actionable line from being silently dropped.
- `write_proposal` (boolean): `true` when at least one `field_updates` entry was
  produced, signaling a downstream workflow to draft a proposal. The skill itself
  performs no live CRM write.

## Example — natural phrasing

**Input:**
```json
{
  "transcript": "They have got around $75k to spend this quarter, so I am moving them to qualified. I will send over a proposal by Friday.",
  "crm_schema": "{\"fields\": [\"budget\", \"status\", \"next_step\"]}"
}
```

**Output:**
```json
{
  "takeaways": [
    "Identified budget: $75k",
    "Identified status: qualified",
    "Identified next step: send over a proposal by Friday"
  ],
  "field_updates": {
    "budget": "$75k",
    "status": "qualified",
    "next_step": "send over a proposal by Friday"
  },
  "needs_review": [],
  "write_proposal": true
}
```

## Example — needs review

**Input:**
```json
{
  "transcript": "We talked budget but they were cagey about the exact number. We should update their status after legal reviews the contract.",
  "crm_schema": "{\"fields\": [\"budget\", \"status\", \"next_step\"]}"
}
```

**Output:**
```json
{
  "takeaways": [],
  "field_updates": {},
  "needs_review": [
    { "field": "budget", "line": "We talked budget but they were cagey about the exact number.", "reason": "References budget with an actionable cue but no clear value could be extracted" },
    { "field": "status", "line": "We should update their status after legal reviews the contract.", "reason": "References status with an actionable cue but no clear value could be extracted" }
  ],
  "write_proposal": false
}
```

## Limitations

- Extraction is heuristic, not semantic understanding. It targets the common CRM
  field shapes above (money / status / next step / simple `field is value`); it
  will miss values expressed in phrasings outside those heuristics. By design,
  such lines surface in `needs_review` rather than being dropped.
- The status vocabulary is a fixed common-CRM set; a bespoke pipeline stage not in
  the vocabulary is reported via `needs_review` when referenced with a cue.
- `crm_schema` must contain a non-empty `fields` array; a missing or malformed
  schema is refused.
- The skill proposes updates only. It performs no live CRM or connector write;
  `write_proposal` gates a downstream workflow.
