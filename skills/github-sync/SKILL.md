---
name: github-sync
description: Plan or execute a scoped GitHub issue, thread, or pull-request synchronization through a configured Runx Connect grant, with approval and readback on writes.
runx:
  category: ops
---

# GitHub Sync

Define one bounded synchronization between local Runx state and a GitHub
repository. The skill makes direction, resource set, filters, content identity,
scope, cursor, and approval posture explicit before a provider adapter is
allowed to read or mutate GitHub.

The default runner is deterministic planning. The `pull` and `push` runners
execute that same plan through Runx's native provider boundary. Pulls need no
human approval because they are bounded reads. Pushes stop at an approval bound
to the exact digest-addressed mutation set, use a stable idempotency key, and
close only on GitHub provider readback. No GitHub token enters this package.

## Composes

- `data-store#append_event`
- `data-store#read_projection`

## When to use it

Use `github-sync` when an operator needs a reproducible pull or push contract for
issues, pull requests, or threads—especially when cursor state must survive
across runs. Use `pull` when the configured Connect grant may read repository
state and `push` only after the desired mutation payloads have been stored under
the digests carried by the plan. Use `issue-triage` to decide what an issue
means and `issue-to-pr` to govern an implementation lane.

Do not use a plan as evidence that remote state was read or changed. Only the
native `runx.provider.operation.v1` result from `pull` or `push` is provider
evidence. Do not silently turn a denied push into a pull.

## How it works

1. Validate the exact `owner/name` repository, direction, resource kind, bounded
   filters, maximum result count, and requested scope.
2. A `pull` plan requires read scope and records the exact resources a provider
   may fetch.
3. A `push` plan requires write scope and digest-only mutation payloads. It
   returns `ready_for_approval`; planning itself does not open or satisfy that
   gate.
4. Optional `plan_and_append_cursor` composes the canonical `data-store` skill
   to append the bounded plan and read the projection back. That proves local
   cursor persistence, not GitHub synchronization; this package does not own a
   second cursor database or storage adapter.
5. `pull` executes the plan with `repo.read`. `push` requests approval, executes
   with `repo.write`, and binds the stable idempotency key. Both use the native
   provider lane and preserve its readback packet.

## Inputs and result

- `repo` is exactly `owner/name`.
- `direction` is `pull` or `push`.
- `resources` contains the issue, PR, or thread selector and a limit no greater
  than 100. Push mutations carry stable resource refs and SHA-256 content
  digests rather than unbounded bodies.
- `scope` is the requested read or write scope.

The `runx.github_sync.v1` plan records exact direction, provider operation,
scope, filters, blockers, approval posture, cursor state when used, and the
explicit absence of remote effects. Execution additionally emits the native
provider-operation packet; it does not rewrite the planning packet to imply a
remote effect.

## Stop conditions

- Refuse malformed repositories, unknown resource kinds, unbounded selectors,
  limits above the contract, or mutable payloads without stable refs and
  digests.
- Refuse push when write scope is missing; do not degrade silently.
- Do not treat local cursor persistence as remote provider readback.
- Refuse a missing, ambiguous, wrong-provider, or under-scoped GitHub Connect
  grant rather than falling back to a raw token or package HTTP client.
- Do not claim comments, labels, issues, or PRs were read or changed until the
  native provider operation proves the expected access, operation, and readback.

## Example

A caller wants to pull the next 50 open issues after cursor `abc`. Planning can
persist that cursor locally; `pull` then performs `issues.read` through the
configured grant and returns the provider result. A label push carries only the
stable resource ref, operation, and content digest through planning. `push`
requests approval for that exact set and retries under one idempotency key.
