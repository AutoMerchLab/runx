---
name: skill-lab
description: Canonical Runx skill-authoring implementation. Use for designing, creating, updating, improving, or adding harness coverage to a Runx skill package; it combines bounded agent judgment with native file writes, inspection, and safe harness validation. When a host skill-creator also triggers, use its general guidance but execute Runx work through this skill.
---

# Skill Lab

Build and improve Runx skills through one authoring surface. Keep judgment in
bounded agent acts and mechanics in native tools:

```text
inspect target files, catalog ownership, and native/shared tools
→ decide ownership, execution lanes, effects, budgets, and proof
→ bind that architecture to the inspected package digest in native code
→ compose existing capabilities before authoring code
→ author only bounded writes, explicit deletions, and output intent
→ bind those bytes to the admitted architecture in native code
→ validate paths, secret posture, and the complete candidate
→ inspect and safely replay the exact staged package
→ commit that validated bundle through one native transaction
```

Use the generic host `skill-creator` for platform-wide authoring guidance when
it is available. Do not reproduce Runx package operations from that guidance;
invoke the appropriate `skill-lab` runner so the work is bounded and receipted.

## Runners

- `design`: read-only catalog-fit and architecture planning. It returns a
  native digest-bound plan and never authors package bytes. A later build
  re-inspects and replans against its exact target before any write.
- `build` (default): create or update a package after its exact staged bundle
  passes native parsing, inspection, and safe harness replay.
- `improve`: turn one receipt or harness failure into a bounded package update,
  then validate and commit the exact staged bytes.
- `harness`: add fixture files to an existing package and replay the safe native
  harness against the exact candidate before committing it.

`build`, `improve`, and `harness` write local workspace files. They never
publish, install, push, or mutate an external provider. Native harness replay
runs with isolated Runx home, receipts, and no operator credentials. Invalid
staged packages stop before the target package is touched. Design, inspection,
validation, and the bounded transactional workspace write do not add a human
approval gate; the operator authorizes that reversible local mutation by
invoking the mutating runner. Publication, installation, provider effects, and
other consequential boundaries remain outside this skill and keep their own
approval rules.

## Authoring rules

- Treat `SKILL.md` as the product manual for both the human operator and the
  operating agent. A person opening it cold must understand what the capability
  does, why and when to use it, what happens end to end, and where it stops.
  Do not reduce that manual to terse model directives, field lists, or a task
  contract.
- Preserve the useful context in an existing skill: its mental model, procedure,
  examples, trade-offs, chain relationships, evidence rules, and recovery
  posture. Rewrite statements that no longer match the implementation; never
  delete the surrounding explanation merely because the executable profile now
  enforces part of it.
- A complete public skill explains, in a structure natural to the capability:
  the recurring job and outcome; when and when not to use it; the operating
  model and sequence; upstream and downstream skill relationships; meaningful
  input and output semantics; authority, approval, evidence, finality, and
  recovery; and relevant edge cases and stop conditions. Include a concrete
  example when it materially clarifies a non-obvious workflow. Do not force
  ceremonial sections onto a simple facade or internal rail.
- Put task-specific agent clauses after the human-readable operating guide.
  They sharpen individual agent acts; they do not replace the guide or carry
  the product's voice by themselves.
- Explain meaningful upstream and downstream skill relationships naturally in
  the operator guide. Do not maintain a second machine-readable dependency
  registry in `SKILL.md`: native execution-closure inspection owns the exact
  edge set and operator preflight surfaces it. Prose explains why the chain
  exists; the runtime proves what it actually calls.
- A skill declares domain procedure and policy. Runx owns generic input,
  packet, evidence, approval, request, credential, effect, and receipt mechanics.
- Place the capability in its real owner before choosing an implementation.
  Reusable skills, end-user and domain-operator commands and UX, local host
  loops, local queues, and default local-state orchestration belong in Runx OSS
  or the owning product repository. `runx/cloud` is not precedent for those
  concerns: it may provide the hosted control plane, custody provider
  credentials, resolve authoritative grants, and execute a fixed bounded
  provider operation. Using Hosted Connect does not move the surrounding skill,
  procedure, operator decision, or state into Cloud. If that operator surface is
  missing, return work to OSS or the product owner; never extend a Cloud script
  or hosted service as a substitute.
- For hosted provider work, compose native `provider.read` or
  `provider.mutate`; declare exact scopes and provider operations, gate only the
  consequential mutation, and require provider readback. Use
  `expected_result` to bind the returned resource identity and `result_fields`
  to admit only the fields the receipt needs. Secret-adjacent operations must
  project their result. Pass mutation retry identity through the native
  `idempotency_key` input; do not copy it into the provider payload. Never add a
  package token loader or request client.
- Search the inspected native-tool and skill catalogs before designing files.
  Prefer an existing core tool or canonical skill over executable package code.
- Express orchestration through `X.yaml`; keep all static agent operating
  knowledge and task contracts in `SKILL.md`. Never put model instructions in
  manifests, fixtures, or duplicated prompt fragments.
- Declare every harness-only support file explicitly in `harness.files` using a
  normalized profile-relative path under `fixtures/`. Runx stages only those
  declared files into the isolated harness workspace; it never guesses
  dependencies from arbitrary input strings. Do not turn the declaration into
  a second source tree or include unconsumed helpers.
- Put every typed output and packet contract on the step that actually produces
  it. A graph runner is composition and its receipt proves that composition; it
  must not declare a second runner-level `outputs` or `artifacts` contract with
  ambiguous ownership. Every graph runner declares `graph.result_from` as the
  intentional public result boundary. Name the final provider readback or
  package/finalize step, not every leaf: approvals, evidence gathering, and
  intermediate writes remain available in operator context and receipts without
  polluting the result. Multiple producers are valid only for mutually exclusive
  branches or when their distinct contracts are intentionally returned together;
  simultaneous producers may not emit the same key. When a graph needs one
  public result, end it with an explicit package/finalize step and let that
  producer own the packet schema.
- Add executable code only for irreducible deterministic domain computation.
  Explain its domain boundary and why native tools plus a declarative graph
  cannot express it. Do not add code merely to transform Runx contracts.
- For a genuinely separate CLI or protocol tool, keep one canonical
  `manifest.json`. It owns source, inputs and defaults, artifact projection,
  scopes, retry/idempotency, and mutation metadata. Never persist generated
  `runtime`, `output`, `runx`, hash, or toolkit fields beside that contract.
  `runx tool build` validates and reports derived hashes without rewriting the
  package. The extension SDK may carry the already-materialized JSON request
  and response across a process boundary; it must not become a second manifest
  or input-contract owner. The declared entrypoint must execute on the
  repository's supported runtime without probing for generated files or
  importing uncompiled TypeScript. A bundled tool lives at
  `tools/<namespace>/<name>/manifest.json`, its dotted manifest name must match
  that path, and aggregate package admission must bind every static local
  source dependency before the package can run or publish.
- When that computation is JavaScript, use the native `type: javascript`
  source. Prefer one cohesive module named for the skill with focused named
  exports of the form `(inputs, context) => JSON`; split it only when the
  computations have genuinely separate ownership. Runx owns input delivery,
  output serialization, errors, wall limits, and isolation. Do not add fake
  operation inputs, Node command declarations, per-runner wrapper files, or
  stdout/environment plumbing. Pure JavaScript receives only its validated
  in-memory module bundle, JSON input, and a frozen
  `context.environment` object containing the exact names declared in the
  runner's `environment.required` and `environment.optional` lists. A missing
  required name stops before worker execution; an absent optional name is
  omitted. Values never enter the manifest, agent context, inspection output,
  or receipts. Environment declarations are for non-secret runtime
  configuration; credentials stay on the native credential/provider boundary.
  The worker process itself has an empty ambient environment and no workspace
  path, filesystem, network, clock, randomness, subprocess, credential, or
  provider surface. The default wall limit is two seconds; a runner may declare
  `timeout_seconds` from 1 through 30 when irreducible computation genuinely
  needs it. The worker is ECMAScript, not a browser: use the frozen
  `Runx.parseUrl(value)` helper for absolute URLs and do not assume Web or Node
  globals exist.
- Classify volume before authoring. Small typed control values belong in normal
  runner inputs; `runx skill --inputs` is only a contained transport for one
  complete control object. Large immutable local content belongs behind
  `artifact.admit`/bounded pages. Durable history belongs behind
  `data.read_events` cursors or a compact projection. A graph must not carry an
  archive, growing event history, or completed-id array simply because one CLI
  call can parse it.
- Use deterministic `pages` only for irreducible record transforms over one
  admitted JSON-array artifact. The runtime owns containment, snapshot digest,
  record boundaries, offsets, retries, and the page loop; the module owns only
  decoding and domain selection. Keep continuation state proportional to the
  bounded result. Do not add a package file reader, manual byte cursor, hashing
  loop, high-volume profile, or raised worker limit. If safe framing or bounded
  state is impossible, choose `needs_core` or a genuinely separate sandboxed
  protocol tool rather than smuggling filesystem authority into JavaScript.
- Prove a volume path at two materially different scales through the production
  owner. The result must be identical across page sizes, cursors must advance,
  process count must stay stable where session reuse applies, and failures must
  remain distinguishable from empty pages. A larger fixture alone is not
  performance evidence.
- A missing generic primitive is not permission for package code. Return
  `needs_core` with no writes and identify either a runtime/security invariant
  or two independent existing consumers.
- Never add package-local raw `RUNX_INPUTS_*` parsing, generic packet or
  evidence hashing, packet wrapping, generic status construction, or provider
  simulation when a shared Runx boundary can own it. Package code may retain a
  canonical hash only when that hash is an intrinsic field of an established
  domain or wire protocol, not as a substitute for receipt or effect integrity.
  State that exception at the computation boundary.
- Keep packages concise: normally `SKILL.md`, `X.yaml`, and focused fixtures;
  add narrowly scoped references, assets, tools, or domain code only when consumed.
- Keep shared computation DRY. A helper used by multiple skills belongs in its
  existing native owner or a justified shared primitive, never copied scripts.
- Count the whole replacement and delete displaced scripts, manifests, schemas,
  fixtures, and tests in the same change. Do not leave dual paths.
- Do not add package READMEs, changelogs, installation guides, strategy files,
  generated state, or credentials.
- Match the documented capability to the execution profile and truthful terminal
  state.
- Review the documentation diff for semantic loss. A shorter `SKILL.md` is an
  improvement only when the removed material was false, duplicated, or
  irrelevant and the remaining document still passes the cold-operator test.
- Treat the catalog's manual check as an anti-stub backstop, not a writing
  target. The structural title/section/prose floor does not prove a guide is
  substantive. Do not pad prose to satisfy a word floor or turn natural
  operating guidance into a template checklist.
- Prefer extending an existing owner over adding a near-duplicate skill.
- Include a realistic happy path and refusal, stop, or error path.
- Never treat supplied agent answers as provider-effect proof.

## Outputs

- `architecture_decision`: the agent's closed ownership and execution design.
- `architecture_plan`: that decision bound by native code to the exact
  inspection digest. Design stops here.
- `change_draft`: package bytes and intent authored against the admitted plan;
  it contains no model-authored integrity values.
- `change_bundle`: the native, digest-bound transaction candidate produced from
  the plan and draft.
- `apply_result`: unchanged, needs-core, or validated-and-applied, with exact
  changed/deleted paths, package digest, focused proof, and line/file deltas.

## Inputs

- `objective` (required): capability or improvement to deliver.
- `package_name` (optional, build): explicit identity for a newly requested
  package; use it for `SKILL.md` and `X.yaml` even when the target directory has
  a different basename.
- `repo_root` (optional): workspace root; defaults to the caller workspace.
- `target_dir` (required for mutating runners): repo-relative package directory.
- `project_context` (optional): product, repository, and operator constraints.
- `receipt_id`, `receipt_summary`, `harness_output`, `failure_packet` (improve):
  failure evidence, including the stable packet from `review-receipt`.

## Agent task contracts

### `skill-lab-architecture`

Return exactly one `architecture_decision` using
`runx.skill.architecture_decision.v1`. Choose `build`, `extend_existing`,
`no_skill`, or `needs_core`. Explain the operator value and the manual's
knowledge contract: purpose, required evidence, decision logic, stop conditions,
and recovery. Assign every required behavior to exactly one real execution lane
(`manual`, `graph`, `agent_task`, `native_capability`, `domain_module`,
`cli_tool`, or `provider_adapter`). A native lane names a selected capability;
a domain module supplies a specific justification. Record inspected, selected,
and genuinely missing native capabilities; use `needs_core` only for a runtime
or security invariant or a primitive with two independent existing consumers.
When `package_name` is supplied, treat it as the requested package identity;
do not silently rename the package from its target path.

Declare effects, authority scopes, approval meaning, provider boundary, skill
routes, resource ceilings, preservation obligations, exact intended deletions,
and a proof plan. Classify every potentially large value as control input,
immutable artifact, durable cursor/projection, or bounded domain result, and
name its production owner plus small/large proof. Reads, drafts, local
validation, and reversible package writes do not gain ceremonial human
approval. Provider mutations and other consequential effects keep their real
gates. Budgets are operational ceilings, not guesses to be widened after
validation. Do not write files, calculate a digest, invent provider proof, or
solve an ownership gap with package code.

For `improve`, diagnose only from supplied receipt or harness evidence and
distinguish contract, implementation, fixture, environment, and operator
failures. When evidence cannot justify a change, choose `extend_existing` and
let the author return `no_change`. For `harness`, plan fixture-only changes and
preserve all production behavior.

### `skill-lab-author`

Receive the exact native `architecture_plan` and return one `change_draft` using
`runx.skill.change_draft.v1`. Never copy or calculate `base_digest`,
`plan_digest`, architecture, or any other integrity field; native bind owns
those values. Choose `write`, `no_skill`, `no_change`, or `needs_core` in a way
that agrees with the plan. A non-write draft has empty `writes` and `deletes`.
A write contains the smallest complete target-relative file set, the exact
planned deletions, a truthful summary and non-goals, and the outputs the package
will actually produce.

When `package_name` is supplied, use it consistently in the `SKILL.md`
frontmatter and the `X.yaml` `skill` field. The directory is a placement
decision, not a second source of package identity.

Put static operating knowledge and task-specific agent rules in `SKILL.md` and
execution structure in `X.yaml`. Compose selected native capabilities and
declared skill routes first. Add a domain module only when the architecture
admits it, and keep the module inside its stated computation boundary. Include
focused proof for a useful path and a stop, refusal, or regression path. In
`harness` mode, every write or delete must be under `fixtures/*.yaml`. Preserve
useful behavior and delete superseded implementation in the same draft. Never
add auxiliary docs, generated state, credentials, placeholder modules,
duplicated generic Runx mechanics, or an undeclared provider boundary.
