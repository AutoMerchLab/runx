---
name: prior-art
description: Inspect bounded local repository and Runx catalog evidence before a design, draft, or operator decision, then produce citation-bound findings and a reuse, amendment, new-work, or stop recommendation. Use when existing tools, skills, standards, or project patterns must constrain downstream work; use research or web-fetch first for external sources.
---

# Prior Art

Ground the next decision in sources that were actually inspected.

## Procedure

1. Supply a bounded `objective`, optional work-plan `decomposition`, and any repo-relative `source_paths` that matter.
2. The deterministic inspection step indexes the local Runx skill catalog and hashes bounded requested files. Missing, escaping, oversized, or non-file sources are recorded rather than guessed around.
3. Read only the indexed sources needed for the objective. State each finding as `claim`, `source`, `relevance`, and `confidence` (`verified`, `likely`, or `unverified`).
4. Name adjacent catalog skills and the boundary each already owns. Recommend `reuse`, `amend`, `new_work`, or `stop`; do not create a duplicate primitive because an existing package is imperfect.
5. The deterministic finalizer checks every verified citation and adjacent skill against the inspection index. Unsupported verified claims or missing requested sources force `needs_more_evidence`.

External URLs are not inspected by this runner. Fetch them through a governed source skill, persist the bounded evidence in the workspace, then pass its path here.

## Output

```yaml
decision: ready | needs_more_evidence
findings:
  - claim: string
    source: repo-relative path
    relevance: string
    confidence: verified | likely | unverified
catalog_fit:
  decision: reuse | amend | new_work | stop
  adjacent_skills: array
  rationale: string
recommended_flow: array
quality_bar: object
sources: array
risks: array
evidence: object
validation: object
```

Inputs are `objective`, optional `decomposition`, `graph_purpose`, `audience`, `artifact_contract`, and bounded `source_paths`.
