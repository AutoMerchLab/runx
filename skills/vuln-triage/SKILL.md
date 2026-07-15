---
name: vuln-triage
description: Triage dependency or ecosystem vulnerability risk into remediation and advisory packets, grounding on verified CVE evidence when it is supplied.
runx:
  category: security
---

# Vulnerability Triage

Take a dependency surface or project scope and produce a bounded security
packet: what is affected, how serious it is, what to do next, and whether a
public advisory is justified. This is the judgment layer, for operator-facing
risk analysis, remediation planning, and advisory drafting. It is not a license
to run arbitrary destructive scans.

When `cve_evidence` is supplied (the verified, exact-version findings a
`cve-audit` run seals), triage those confirmed facts rather than inferring
exposure. That is the intended chain: deterministic evidence from `cve-audit`
flows into this judgment, and the drafted advisory flows on to `vuln-disclosure`.
Without it, analyze the target surface directly.

Cite package data, versions, advisories, scan output, commits, or other concrete
evidence for every exposure claim. Separate confirmed exposure from possible
risk and write calmly: no vague severity, alarmism, or public disclosure claim
without evidence and authorization. Return `needs_more_evidence`,
`needs_human`, or `do_not_publish_advisory` when affected versions, exposure,
remediation, or disclosure posture cannot be verified.

## Output

Scan runner:

- `dependency_inventory`: affected components and versions.
- `advisories`: findings with severity, exposure, and evidence.
- `remediation_plan`: concrete next actions.
- `operator_summary`: concise decision-ready summary.

Advisory runner:

- `advisory_draft`: public or maintainer-facing advisory text.
- `maintainer_summary`: concise summary for repo owners.
- `disclosure_checklist`: what to verify before public release.

## Inputs

- `target` (required): repo, lockfile, package set, or ecosystem slice.
- `objective` (optional): what the operator wants from this scan.
- `scan_context` (optional): known packages, incidents, or prior findings.
- `advisories` (optional): structured findings from the scan runner.
- `remediation_plan` (optional): structured remediation plan for the advisory pass.
