---
name: CRM Cleanup
description: Extracts structured, actionable CRM updates from natural language meeting transcripts.
---

# CRM Cleanup

The `crm-cleanup` skill processes meeting transcripts and maps spoken action items into structured CRM fields. It uses dynamic NLP patterns to populate fields defined dynamically in the `crm_schema`.

## Overview

Sales representatives often leave a meeting with a messy transcript. This skill helps automate the data entry process by extracting field values like budget, status, and next steps, directly mapping them to a required CRM structure.

## Inputs

- `transcript` (string, required): The raw or lightly-edited text transcript from the meeting or call.
- `crm_schema` (string, required): A JSON string containing a `fields` array of allowed field names (e.g. `{"fields": ["budget", "status"]}`).

## Outputs

- `takeaways` (array): A list of sentences describing the exact information that was extracted from the transcript.
- `field_updates` (object): Key-value pairs representing the CRM field names and the parsed values.
- `write_proposal` (boolean): `true` if any actionable fields were found, signaling downstream workflows to trigger a proposal draft.

## Example Usage

**Input:**
```json
{
  "transcript": "The client confirmed the budget is $10k and the next step is to send a proposal.",
  "crm_schema": "{\"fields\": [\"budget\", \"next_step\"]}"
}
```

**Output:**
```json
{
  "takeaways": [
    "Identified budget: $10k",
    "Identified next step: to send a proposal"
  ],
  "field_updates": {
    "budget": "$10k",
    "next_step": "to send a proposal"
  },
  "write_proposal": true
}
```

## Limitations
- Relies on structured regex matching ("X is Y") to find fields. Advanced contextual sentiment extraction is not supported.
- `crm_schema` must explicitly contain the `fields` array. Missing or malformed schemas will result in a refusal.
