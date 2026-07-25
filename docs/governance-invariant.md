# The uniform-governance invariant

Every governed execution in runx passes through the same four stages, in order:

```
admit  ->  deliver credentials  ->  sandbox  ->  seal
```

This is the contract that makes "governed execution layer" true rather than
aspirational: a front cannot run an act without being admitted, cannot leak
ambient secrets, cannot escape its declared sandbox, and cannot finish without a
signed receipt that attests what was authorized. Adding a new front does not get
to opt out of any stage.

Two stages are enforced **structurally** by the graph-step orchestration, so a new
step type gets them for free and cannot regress them. Two are **adapter contracts**
every front honors in its own execution path.

## The stages

### 1. Admit (structural)

Before any step handler runs, the orchestration admits the act against the
configured authority and effects: `enforce_step_authority_admission`
(`crates/runx-runtime/src/execution/runner/authority.rs`), called once per step at
`crates/runx-runtime/src/execution/runner/steps.rs:230` before dispatch. Local
skills admit through `admit_local_skill` in the core policy crate
(`crates/runx-core/src/policy/local.rs`). An unadmitted act never reaches a handler.

### 2. Deliver credentials (adapter contract)

The resolver turns a runner's declared requirement into `CredentialDelivery`.
Adapters receive that delivery separately from ambient configuration, inject
only its declared secret environment bindings into the child boundary, and
redact material from captured output before projection. The cli-tool front does
this in `crates/runx-runtime/src/adapters/cli_tool.rs`; HTTP substitutes only
`${secret:NAME}` header references. A runner's `environment` declaration is
not a credential transport.

### 3. Sandbox (adapter contract)

The adapter resolves the declared sandbox profile to a platform runtime and wraps
the command in it: `resolve_sandbox_runtime` plus the command wrapping in
`crates/runx-runtime/src/sandbox/command.rs` (bubblewrap on Linux, sandbox-exec on
macOS, fail-closed `DeclaredPolicyOnly` when no backend enforces and enforcement is
required). The resolved sandbox is recorded in the output metadata
(`crates/runx-runtime/src/sandbox/metadata.rs`) so the receipt attests it.

### 4. Seal (structural)

After the handler returns, the orchestration seals the step centrally:
`run_registered_step` overrides the receipt's admission witness via the single
`step_admission_witness` helper (both in
`crates/runx-runtime/src/execution/runner/steps.rs`), recording which authority
admitted the act (or a local-runtime witness when none was admitted). Because the
witness is set in one central place rather than per handler, a new step type cannot
seal without it.

## Adding a front

A new graph-step front (a new entry in the step-type registry) inherits **admit**
and **seal** from the orchestration automatically. It must honor the **credentials**
and **sandbox** contracts in its own adapter, exactly as the cli-tool front does.
The structural stages cannot be bypassed; the adapter-contract stages are the
front author's obligation and are covered by the conformance tests below.

## Conformance

| Stage | Guarding test |
| --- | --- |
| Admit + Seal | `crates/runx-runtime/tests/governance_witness.rs` (an admitted step records its authority in the sealed witness; an unadmitted step falls back to local-runtime) |
| Deliver credentials | `crates/runx-runtime/tests/credential_delivery.rs`, `credential_grant_policy.rs` |
| Sandbox | `crates/runx-runtime/tests/cli_tool_contract.rs` (enforced-readonly) |

The seal stage was made structural and uniform across step types in the runtime
runner; the broader contract (this document) is the operative statement of the
invariant for new fronts.
