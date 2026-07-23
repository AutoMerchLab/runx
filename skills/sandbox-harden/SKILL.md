---
name: sandbox-harden
description: Compile source-bound workload requirements into the narrowest real Runx sandbox declaration and report controls that require another runtime boundary.
runx:
  category: security
---

# Sandbox Harden

Choose the narrowest sandbox Runx can actually express for one identified
workload. The important word is *actually*: this skill plans against the same
native declaration and admission policy used by Runx execution. It does not
emit an impressive-looking seccomp or container profile that the runtime cannot
enforce.

The result is a plan, not an applied sandbox. It records `applied: false`; the
later execution boundary owns enforcement and runtime readback.

## What Runx can govern

Runx sandbox declarations control:

- one of `readonly`, `workspace-write`, or `network` profile;
- workspace-relative writable paths;
- environment-variable allowlisting;
- working-directory policy; and
- whether enforcement is required.

They do not currently express syscall/seccomp policy, host-level egress
allowlists, Linux capability sets, or CPU and memory quotas. Those controls are
reported as unsupported and must be owned by a container, host, orchestrator,
or another explicit runtime boundary. Network plus workspace write is refused
because no least-privilege Runx profile represents that combination.

## When to use it

Use `sandbox-harden` before executing a third-party skill, image, CLI, batch job,
or other workload whose filesystem, environment, and network needs must be
reviewable. It is also useful when replacing a broad default profile with a
source-bound declaration.

Do not use it to run the workload, audit API scopes after execution, or claim
host controls were installed. `least-privilege` reasons about exercised
authority; `audit-receipt` reviews a sealed run after the fact.

## How it works

1. Identify the workload by exactly one immutable image digest or skill
   reference.
2. Supply fresh requirements with a stable source reference, SHA-256 digest,
   observation time, network need, writable paths, environment allowlist,
   working-directory policy, and enforcement posture.
3. Optionally supply a baseline that caps network, writable paths, environment,
   and enforcement. The plan may narrow that baseline but cannot silently
   broaden it.
4. The native planner validates provenance and freshness, rejects absolute write
   paths and unsupported controls, selects the narrowest real profile, and runs
   core sandbox admission.
5. The packet records the declaration, admission reasons, unsupported controls,
   validation findings, and residual risk.

## Inputs and result

- `workload` contains exactly one `image_digest` or `skill_ref`, an optional
  class, and a `requirements` object with source-bound runtime needs.
- `as_of` and `max_age_days` define deterministic freshness.
- `baseline` may forbid network, require enforcement, and bound allowed writable
  paths and environment names.

The `runx.hardening.v1` result is `ready`, `needs_more_evidence`, `refused`, or
`unsupported_runtime_shape`. A ready result includes the normalized Runx
declaration and core admission result. Residual risk remains at least the risk
of controls Runx does not express; it is never proof that the host is hardened.

## Stop conditions

- Stop when workload identity is missing, ambiguous, or uses an invalid image
  digest.
- Reject missing, malformed, stale, or future-dated requirements provenance.
- Refuse absolute writable paths and any request broader than the supplied
  baseline.
- Return `unsupported_runtime_shape` for network plus workspace writes or for
  requested syscall, host-egress, or capability controls.
- Never copy secret values into the requirements or result; name only allowed
  environment variables or opaque handles.
- Do not claim the declaration was applied or the workload successfully ran.

## Example

A source-bound batch job needs no network, writes only `tmp/results`, reads
`REPORT_FORMAT`, and requires enforcement. The planner selects
`workspace-write`, admits the relative path and environment name, and reports
host syscall and resource limits as separate residual controls. If the same job
also requests network, it returns `unsupported_runtime_shape` rather than
quietly granting a broad network-and-write sandbox.
