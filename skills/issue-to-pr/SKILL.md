---
name: issue-to-pr
description: Implement one bounded repository issue with normal host tools, prove the tested change through one scafld finalize wall, and optionally publish an approved pull request with provider readback.
runx:
  category: code
---

# Issue to PR

Turn one issue into a tested change and, when authorized, a verified pull
request. Use the repository's normal development workflow. Runx owns evidence
continuity and the consequential publication boundary; it does not replace the
host agent's editor, shell, Git, tests, or repository judgment.

## Direct operator flow

1. Resolve the repository from an explicit `owner/name` or the current
   checkout's `origin`. Never infer the target from an available grant.
2. Use the already-authenticated local `gh` and Git paths for inspection.
   Hosted Connect is a compatible fallback or explicit operator binding, not
   the default reason to leave local tooling.
3. Run one preflight before work: issue identity, repository permission, branch
   state, required tools, requested outcome, and publication authority.
4. Investigate, edit, and test with the host agent's ordinary tools. Do not
   manufacture Runx answer files between normal coding steps.
5. Call `scafld finalize` exactly once after the change and tests are ready.
   Preserve its signed receipt reference and contract digest in `host_result`.
6. If PR publication is not authorized, return the tested/finalized result and
   stop. Do not silently downgrade the work to a plan.
7. If publication is authorized, pass the exact `host_result` to `publish`.
   Runx approves one `pullrequest.open` mutation and independently reads the PR
   back. Notifications, feeds, issue comments, and documentation sync are
   optional downstream skills.

## Reuse in chains

- `from-evidence` accepts `runx.issue_to_pr.issue_evidence.v1` and skips GitHub
  discovery and issue read.
- `resume` accepts both prior issue evidence and a completed
  `runx.issue_to_pr.host_result.v1`; it does not repeat host work or finalize.
- `publish` accepts the completed host result and performs only the approved PR
  mutation plus readback.
- Preserve the same idempotency key across pause, retry, and resume. An
  uncertain PR creation never gets a new key.

## Host work contract

The `issue-to-pr-host-work` act performs normal repository work and returns:

```yaml
host_result:
  schema: runx.issue_to_pr.host_result.v1
  status: completed | blocked | failed
  repository: owner/name
  issue_number: string
  branch: string
  files: [relative/path]
  tests:
    - command: string
      status: passed | failed
      evidence: string
  finalization:
    status: passed | failed
    receipt_ref: string
    contract_digest: sha256:...
    invocation_count: 1
  publication:
    decision: hold | ready
    title: string
    body: string
    head: string
    base: string
    draft: boolean
    idempotency_key: string
  errors: [string]
```

Do not claim `completed` without a real edit/test outcome and one successful
finalize result. Do not claim `published` from a branch push, API
acknowledgement, or draft packet; only independent PR readback closes that
state.

## Stop conditions

- Wrong or ambiguous repository: stop with one target-resolution blocker.
- Missing local auth and no compatible hosted grant: return the exact `gh auth
  login` or `runx connect` handoff; do not cycle through unrelated skills.
- Dirty or conflicting branch state: stop before mutation and preserve the
  evidence already gathered.
- Failed tests or failed/stale finalize: return blocked or failed, never
  succeeded.
- Missing publication approval: retain the tested finalized change locally and
  stop before PR creation.
- Provider failure after approval: preserve the approval, idempotency key,
  finalization evidence, and mutation recovery state. Resume at publication;
  do not repeat issue discovery, coding, tests, or finalize.
