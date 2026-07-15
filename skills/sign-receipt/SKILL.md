---
name: sign-receipt
description: Prepare an evidence-bound attestation of an off-runtime action for a signed Runx receipt; never claims an external ledger append without adapter evidence.
runx:
  category: security
---

# Sign Receipt

Turn an action that happened outside a run into an attestation request a signed
Runx receipt can carry.

## What this skill does

`sign-receipt` binds an actor, a claim, and the evidence that backs the claim
into one attestation request, tests the claim against that evidence, and hands
the runtime a payload it can include in its signed run receipt. The agent never
claims that a signature or ledger append happened; those outcomes belong to the
runtime receipt or a configured ledger adapter.

The runtime already signs every hop of work it executes itself; each act, each
decision, each refusal lands in a sealed receipt without anyone asking. Work that
happens elsewhere has no such record. A human approved a refund in the provider
console. A partner service shipped a build. A reviewer signed off in a tool runx
never touched. That work is real, but to the ledger it is a rumor. This skill
makes the rumor citable.

It will not mark a claim ready when the evidence does not support it. An
attestation with no binding evidence would ask the runtime to seal a guess, and
a sealed guess is worse than no record at all.

**Distinctness:** it attests work that happened OUTSIDE a run; the runtime
already signs every hop it executes itself. `audit-receipt` reads a sealed
in-runtime receipt to check authority; `sign-receipt` prepares a new attestation,
which the runtime seals into a receipt, for work the runtime never saw.

## When to use this skill

- A human or external service performed an action runx did not execute, and a
  later run needs to depend on it with provenance.
- An out-of-band approval, payment, build, or sign-off must enter the ledger so
  an audit can trace it.
- A partner attestation must be normalized into a runx-shaped, signed receipt.

## When not to use this skill

- To audit a run the runtime executed. That is `audit-receipt`, which reads an
  existing sealed receipt rather than preparing a new attestation.
- To execute the action itself. Use the action skill (`spend`, `send-as`,
  `refund`); they seal their own receipts.
- To attest a claim with no evidence, or with evidence you cannot reference
  without inlining secret or personal data.
- To store the evidence content. This skill stores references to it.

## Procedure

1. **Take the claim.** The caller states what was done (`action`), who did it
   (`principal`), and exactly what is being asserted (`claim`). An attestation
   asserts that a specific actor did a specific thing; an unnamed actor cannot be
   attested.
2. **Bind the evidence.** Evidence arrives as references and digests: a provider
   transaction id, a commit sha, a signed approval handle, a content digest.
   Record what each reference proves, never the underlying content. References,
   digests, handles, ids, and spans only; raw content, secret values, card
   numbers, and PII never appear.
3. **Test claim against evidence.** Evidence sufficiency is the gate. The run
   does not reach sealing readiness until each load-bearing part of the claim maps
   to a binding reference. If the references do not support the claim, stop at
   `needs_more_evidence` rather than claiming readiness. If the claim is broader than the
   evidence, narrow it to what the references prove, or stop. The scope of the
   signature is the scope of the claim, nothing wider.
4. **Mark ready for runtime sealing.** On a supported claim, set
   `decision: ready_to_seal` and hand the payload to the runtime. A sealed run
   receipt proves that Runx accepted this payload; it does not prove an external
   ledger append unless a ledger adapter also returns that evidence. Corrections
   are new attestations that reference the prior one, never edits.

## Edge cases and stop conditions

- **Missing action, evidence, principal, or claim:** return `needs_agent`; the
  attestation has no complete subject.
- **Evidence does not support the claim:** return `needs_more_evidence` with the
  specific gap named; do not mark a partial match ready.
- **Evidence carries raw secret or personal data:** record its digest and span,
  drop the raw value; if dropping it removes the proof, return
  `needs_more_evidence`.
- **Claim broader than the evidence:** narrow the claim to what the references
  prove, or stop. A signature must not cover unproven ground.
- **Conflicting references:** stop at `needs_more_evidence`; an attestation must
  not paper over contradiction.

## Output schema

```yaml
attestation:
  decision: ready_to_seal | needs_more_evidence | needs_agent
  action: string          # what was done, in operational terms
  claim: string           # the exact assertion the signature covers
  principal: string       # the actor the attestation names
  evidence_refs:          # bound references, refs and digests only, never content
    - ref: string
      digest: string
      proves: string
  gaps: array             # specific missing or contradictory evidence
  scope:                  # optional, bound on what the attestation may be relied on for
    rely_for: string
```

The runtime receipt carries the payload and runtime-owned receipt identity and
signature evidence. It never carries the evidence content, a secret value, or
any PII drawn from the evidence. A provider or external ledger identifier is
only valid when returned by the adapter that performed that write.

## Worked example

Input: an operator (`ops:jordan`) issued a $40.00 goodwill refund for order
ORD-7741 in the provider console. Evidence is two references: a provider refund
transaction (`provider:refund/re_3PqL2x`, with digest) proving the refund
settled, and a console approval handle (`approval:console-signoff/ap_5512`, with
digest) proving sign-off preceded the refund.

Output: each load-bearing part of the claim maps to a binding reference, so the
gate passes. The attestation names the principal, carries both `evidence_refs`
with their digests, and sets `decision: ready_to_seal`. The scope binds reliance
to downstream dispute and reconciliation runs referencing ORD-7741. Had only
the approval handle been supplied with no settlement reference, the decision
would be `needs_more_evidence`, not a signature claim.

## Inputs

- `action` (required): what was done, off-runtime.
- `evidence` (required): references and digests proving the action, each with
  what it proves. References only; no raw content or secret values.
- `principal` (required): the actor the attestation names.
- `claim` (required): the exact assertion to be attested.
- `scope` (optional): bound on what the attestation may be relied on for.
