---
name: vuln-disclosure
description: Publish a governed, human-approved security advisory from triaged vulnerability risk, gating the outward disclosure behind explicit approval.
runx:
  category: security
---

# Vulnerability Disclosure

This is the outward publication lane: it takes triaged vulnerability risk and
turns it into an approved, public security advisory. Where `vuln-triage` decides
what matters, this skill governs the disclosure to the world, holding the
publish behind an explicit human approval gate.

It builds on `vuln-triage`. Pass `cve_evidence` from a `cve-audit` run to ground
the disclosure in verified, exact-version findings; the deterministic evidence
flows through the triage into the drafted advisory that this skill publishes.

It keeps the security flow bounded and reviewable:

1. produce the risk packet
2. draft the advisory
3. require approval before anything is packaged for publication

Every public claim must trace to the risk packet and a verified advisory source.
State affected scope, impact, and remediation precisely, without alarmism;
speculation stays in private operator notes. If severity, exposure,
remediation, or disclosure authority is unclear, stop at review. If publication
would not materially help affected users, keep the result as an operator
remediation packet instead.

## Output

- `risk_packet`: inventory, confirmed exposure, possible risk, and remediation evidence.
- `advisory_draft`: precise affected scope, impact, and next steps.
- `approval_decision`: disclosure and wording review.
- `publish_packet`: approved advisory and channel metadata.

## Inputs

- `target` (required): repo, lockfile, package set, or ecosystem slice.
- `objective` (optional): what the operator wants from the scan.
- `channel` (optional): final advisory channel; defaults to `advisory`.
- `scan_context` (optional): known incidents or previous findings.
