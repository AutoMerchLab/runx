---
name: x402-pay
description: Validate and pay one x402 challenge through a trusted configured buyer adapter with approval and readback; use x402 explicitly for quote-only planning.
runx:
  category: payments
---

# X402 Pay

`x402-pay` is the discoverable x402 execution boundary over the canonical
`spend` authority model. It validates the exact challenge, performs one
approved payment through a trusted configured buyer adapter, and requires
provider readback.

Runx does not bundle or custody a wallet. Without a compatible tenant-selected
adapter the default blocks with the missing binding. It never asks an agent to
improvise signing or labels a quote or synthetic transaction as settlement.

## Composes

<!-- Generated from the native execution closure; run pnpm core-skills:composes:generate. -->

- `spend#plan`

## When to use it

Use this skill after a paid resource has returned structured x402 payment
requirements and the operator wants that exact challenge paid. Select `x402`
explicitly for preflight, policy review, or quote-only planning.

Use `spend` when choosing among other executable rails. If a trusted x402
adapter is unavailable, stop at the actionable binding blocker or select the
explicit quote runner; never substitute a supposed payment receipt.

## What happens

1. The default delegates validation to `spend:plan`; it does not define a
   second payment policy or authority model.
2. Native `payment.quote` validates the amount, currency, `x402` rail, realm,
   counterparty, operation, limits, and idempotency seed against the complete
   parent `AuthorityTerm`.
3. Runx passes that packet to the selected adapter, gates the exact payment,
   preserves one idempotency key, and requires payment readback. The explicit
   `x402` runner stops after the canonical quote.

A grant id or prose assertion is not authority. The parent term must contain
the actual bounded effect limits and must authorize the same counterparty,
operation, currency, realm, and x402 channel as the challenge.

## The adapter boundary

An eventual execution adapter must implement the standard buyer flow: consume
the server's payment requirements, ask a trusted wallet to create the protocol
payment payload, retry that same paid resource with the signed payment header,
and return resource response plus payment readback bound to the reservation and
idempotency key.

That adapter may run locally with operator-owned credential custody, or through
an explicitly selected hosted Connect grant. In either case:

- the skill and payment workflow remain in OSS;
- the adapter is selected by an opaque profile, never caller-supplied endpoints;
- wallet keys, seed phrases, bearer tokens, and admission material never enter
  skill input or agent context;
- Cloud, when opted into, may resolve the grant and execute one bounded provider
  call but does not own the skill, queue, approval policy, or local state;
- success requires independent paid-resource/payment readback, not an HTTP 2xx
  from a signer or facilitator.

The existing upstream x402 conformance tooling is evidence for the wire
protocol, not an executable adapter for this public skill.

## Inputs and result

- `payment_signal` is the structured x402 challenge with positive minor-unit
  amount, uppercase currency, counterparty, operation, and optional realm.
- `parent_payment_authority` is the complete typed parent `AuthorityTerm`.
- `idempotency_seed` is stable caller-owned material for this intent.
- `realm` optionally narrows the expected authority realm.

The result is a canonical payment quote packet. It never contains a wallet
credential, signature, payment admission token, provider endpoint, or a claim
that funds moved.

## Stop conditions

Stop before output when the challenge is malformed, not x402, over a ceiling,
outside aggregate limits, expired, or mismatched on currency, realm,
counterparty, or operation. Refuse raw credentials and endpoint configuration.
Do not silently select another rail or convert a quote into approval.

For example, if a resource requests USD 1.25 for `search.paid` from
`merchant:demo`, and the supplied parent term authorizes that exact x402 action
up to USD 2.00, the skill emits the bounded quote. It still says nothing was
paid; execution remains blocked on the missing trusted adapter.
