---
name: vault-unseal
description: Prepare a scoped, time-bounded vault-unseal request for approval and adapter execution; never returns a secret or claims a handle was issued.
runx:
  category: security
---

# Vault Unseal

Turn a request for a secret into a reviewable, time-bounded access plan that
hands an approval-ready request to a vault adapter instead of exposing the secret.

## What this skill does

An agent rarely needs a secret. It needs the thing the secret unlocks: one API
call, one signed request, one decrypt for a stated window. This skill plans that
access. It binds the secret reference, the purpose, a TTL, the scope the secret
covers, and the principal asking, then marks the request for a human approval
gate. The output is an unseal plan; it does not contain a handle or an
audit receipt because this agent runner cannot issue either. A configured vault
adapter performs the live unseal after approval and owns its handle and audit
evidence. The secret value is never read into the plan, the receipt, or the
agent's context.

It governs explicit secret access with a TTL and approval; least-privilege
only analyzes scopes, it never touches secrets.

## When to use this skill

- An agent or workflow needs a credential, key, or token to complete one bounded
  task and the access must be approved and audited.
- A break-glass or just-in-time access request needs a plan a reviewer can read
  and a window that expires on its own.
- A downstream action skill needs an approval-ready request that a vault adapter
  can exchange for a handle without putting the secret in agent context.
- An operator wants the planning decision (`ready_for_approval`, `needs_agent`,
  `denied`) separated from adapter-owned execution evidence.

## When not to use this skill

- To read, print, copy, or return a secret value or handle. This skill prepares
  the request; the vault adapter owns live access.
- To grant standing or unbounded access. Every unseal is scoped to one secret
  for one TTL.
- To review or narrow scopes that a subject already holds. Use
  least-privilege for scope analysis against receipts.
- To rotate, store, or mint new secrets. That is a separate vault operation with
  its own gate.
- To bypass the approval gate, widen the scope past the stated purpose, or
  extend a TTL that has already lapsed.

## Procedure

1. Resolve the request. Confirm `secret_ref`, `purpose`, `ttl`, `scope`, and
   `principal` are present. If any required input is missing, stop with
   `needs_agent` and name what is missing. Do not guess a default TTL or scope.
2. Check purpose and scope against policy. Confirm the stated purpose is a
   permitted reason to access this secret, and that the scope does not exceed
   what the purpose needs. If the purpose or scope is not permitted, set
   `decision: denied` and name the policy that refused, not the secret.
3. Set the TTL window. Parse `ttl` into a duration; the window starts at
   approval, not at request. If the TTL is unparseable, absent, or unbounded,
   return `needs_agent`. There is no open-ended unseal.
4. Set the approval gate. A live unseal always requires human approval; set
   `gates.human_approval_required: true`. A complete, permitted request is
   `ready_for_approval`, never evidence that approval or execution occurred.
5. Bind the request. Bind the plan to exactly one `secret_ref` for one TTL
   window under the requested scope. Set `decision: ready_for_approval` only
   when the request is complete and policy-compatible.
6. Hand off execution. After separate approval evidence is attached, a vault
   adapter may consume the plan, issue an opaque handle, and return adapter-owned
   audit evidence. This skill never fabricates either outcome.

## Edge cases and stop conditions

- **Missing required input:** `secret_ref`, `purpose`, `ttl`, or `scope` absent
  returns `needs_agent`. The principal is also required to attribute the access.
- **Purpose not permitted:** set `decision: denied`; name the refusing policy,
  never the secret.
- **Scope exceeds purpose:** narrow to what the purpose needs, or set
  `decision: denied` if it cannot be narrowed safely.
- **Unbounded or lapsed TTL:** refuse. There is no standing unseal and no revival
  of an expired window.
- **Approval absent or denied:** the request remains `ready_for_approval` or
  moves to `denied`; this skill never issues a handle.
- **Caller asks for the raw value:** refuse that part. If the workflow genuinely
  cannot use an adapter-owned handle, return `needs_agent` with the constraint
  named, never the value.
- **Vault adapter unavailable:** return an execution blocker; a plan is not an
  unseal.

## Output schema

The artifact is the `unseal_plan` object, wrapped as `runx.unseal.v1`. The
secret value never appears in any field.

```yaml
unseal_plan:
  decision: ready_for_approval | needs_agent | denied
  secret_ref: string        # reference to the secret, never its value
  purpose: string           # bounded reason the secret is needed
  ttl: string               # bound duration; the window starts at approval
  scope:                    # what the secret unlocks, as stated and as bound
    resource: string
    action: string
    path: string
  principal: string         # who the access is attributed to
  gates:
    human_approval_required: boolean  # always true for a live unseal
  blockers: array           # named reasons the plan is not ready
  execution:
    requires_adapter: true
    requires_approval: true
```

## Worked example

Input: principal `svc/report-exporter` requests `vault://drive/service-account`
for the purpose "sign one Drive export request", `ttl: 10m`, scope
`{ resource: drive.files, action: export, path: /reports/* }`.

Output: `decision: ready_for_approval`; policy permits the purpose and the scope
matches it; `gates.human_approval_required: true`; and
`execution.requires_adapter: true`. After explicit approval, the vault adapter
may issue an opaque handle and records its own audit evidence. The service
account key never enters the plan or the receipt.

## Inputs

- `secret_ref` (required): reference to the secret to unseal, never its value.
- `purpose` (required): the bounded reason the secret is needed.
- `ttl` (required): the access window duration; the window starts at approval.
- `scope` (required): structured statement of what the secret unlocks.
- `principal` (required): who the access is attributed to.
- `policy_notes` (optional): reserved purposes, break-glass conditions, or
  constraints that affect the decision.
- `operator_context` (optional): approval posture or extra guardrails.
