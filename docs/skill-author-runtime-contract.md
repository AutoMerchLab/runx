# Skill Author Runtime Contract

This document defines the lower-level author-visible v1 subprocess ABI for
`cli-tool` skills. Use it only when a capability genuinely needs an executable
or protocol that cannot use Runx's JavaScript module boundary. Ordinary package
domain logic should declare `type: javascript`, export a function from a local
module, accept the resolved input object, and return a JSON-compatible value;
Runx then owns every process detail described below.

The subprocess ABI is shared by the TypeScript adapter while it survives and
the Rust runtime cutover. Internal receipt IDs, artifact IDs, sandbox metadata
internals, and temporary paths are not part of this contract unless named here.

## JavaScript module boundary

A pure package module needs no manifest, wrapper, environment parsing, stdout
serialization, or package dependency:

```yaml
run:
  type: javascript
  module: risk-model.mjs
  export: assessRisk
  outputs:
    assessment: object
```

```js
export function assessRisk(inputs) {
  return { assessment: evaluate(inputs) };
}
```

Omit `export` to select the default export. Module paths are portable relative
`.mjs` or `.js` paths contained by the owning skill. The selected export may be
sync or async. Runx owns the dedicated no-host worker, fixed limits, input and
output framing, timeout, and failure semantics. The module receives neither a
CLI-tool environment nor credentials, and those mechanics are not
package-owned API.

## Volume and artifact pages

Volume changes transport, not execution authority. `runx skill --inputs` may
read one complete control object from a contained UTF-8 JSON file or stdin, but
it does not widen graph, deterministic-worker, output, credential, or approval
limits. Do not pass an archive, history, or growing completed-id list through
that object merely because the CLI can read it.

For one large immutable JSON-array export, declare a paged deterministic source:

```yaml
run:
  type: javascript
  module: archive-selection.mjs
  export: selectPage
  pages:
    path_from: archive_file
    path_scope_from: archive_base
    media_type: application/json
    framing: json_array
    page_bytes: 524288
```

The runtime removes the path and scope fields before module invocation, admits
the contained file to an immutable snapshot, and calls the export repeatedly.
Each call receives `runx_page` with `artifact_ref`, media type, whole digest,
source byte count, page index, exact offsets, range digest, `eof`, complete
encoded records, and the prior continuation `state`. An intermediate result may
contain only `{ runx_page: { state, done? } }`; the final call also returns the
declared domain output. A failed page reports its index and byte offset and
cannot be mistaken for an empty page.

Artifact admission is capped at 512 MiB, a page at 1 MiB, continuation state at
2 MiB, and one execution at 4,096 pages. The normal 4 MiB deterministic-worker
input/output ceilings still apply to each call, so framing also rejects a
single record that cannot fit safely. These are runtime ceilings, not manifest
profiles. Package code must keep continuation proportional to the bounded
result it is building; durable progress belongs in `data.append_event` and is
resumed through `after_version` or a projection rather than an ever-growing
array.

Use `artifact.admit`/`artifact.read` directly when a graph or tool needs exact
byte pages instead of a domain transform. Use `fs.read` and `fs.read_bundle`
only for bounded text. If a format cannot be framed safely or needs genuine
streaming protocol behavior, use one declared sandboxed `cli-tool`; never add
ambient filesystem access to a deterministic module or duplicate the runtime's
containment, hashing, and page loop in package JavaScript.

## Managed-agent tools

An `agent-task` may call only the tools named in its `allowed_tools`. Before the
model runs, Runx resolves every name through the same native-and-local catalog
used for execution. The model receives the catalog description and exact input
schema, including required fields, typed defaults, and
`additionalProperties: false`; Runx does not substitute a permissive guessed
schema. An unresolved allowed tool fails before a provider call.

Invocation then returns through that same catalog path, so a tool cannot be
described from one implementation and executed by another. Native tools retain
their runtime effect, credential, artifact, and receipt boundaries. Local tool
manifests retain the subprocess ABI below. The owning `SKILL.md` and any
declared context-skill manuals provide operating judgment; tool schemas provide
mechanics, not duplicate instructions.

Bundled local tools use
`tools/<namespace>/<tool>/manifest.json`; the dotted manifest name must match
that path. Skill-package admission parses each manifest and binds its complete
static local source closure. Registry publication consumes that admitted set—it
does not rescan or reinterpret tool source—so a missing import, uncompiled Node
TypeScript entrypoint, or path/name mismatch fails before execution or publish.

## Process

The runtime starts the declared command with `shell: false` semantics. Arguments
are resolved before spawn. The skill process runs with piped stdin, stdout, and
stderr. Stdout and stderr are drained completely while the process runs; each
stream retains a bounded 8 MiB prefix without emitting broken UTF-8. The process
supervisor also counts and hashes the complete stream, so a digest-mode native
command can preserve evidence without retaining an unbounded body. Text and JSON
contracts fail closed when their retained body is truncated.

## Environment

The child environment is deny-by-default. The sandbox allowlist admits only
declared host variables plus runtime-authored `RUNX_*` variables.

Guaranteed variables:

- `RUNX_CWD`: the workspace root, resolved as `RUNX_CWD ?? INIT_CWD ?? current_dir`.
- `RUNX_INPUTS_JSON`: serialized inputs when the full input payload is at most 48 KiB.
- `RUNX_INPUTS_PATH`: path to a UTF-8 JSON file when the full input payload is larger than 48 KiB.
- `RUNX_INPUT_<NAME>`: per-input scalar/stringified value when the serialized value is at most 8 KiB.

Input env names are normalized by replacing non-alphanumeric runs with `_`,
trimming separators, and uppercasing. For example, `thread.title` becomes
`RUNX_INPUT_THREAD_TITLE`.

Large per-input values are omitted from `RUNX_INPUT_*`; authors must read
`RUNX_INPUTS_JSON` or `RUNX_INPUTS_PATH` for the full payload.

## Stdin

When `inputMode` is `stdin`, stdin receives the full input object as JSON and
then closes. Otherwise stdin closes without input.

## Cwd Policy

Relative source cwd values resolve from the skill directory. Non-unrestricted
profiles fail closed when cwd escapes the declared policy boundary:

- `skill-directory`: cwd must stay within the skill directory.
- `workspace`: cwd must stay within `RUNX_CWD ?? INIT_CWD ?? current_dir`.
- `custom`: cwd must stay within the skill directory or workspace.

`unrestricted-local-dev` may escape after explicit approval metadata, but the
runtime must not claim approval when no runner approval was supplied.

## Timeout

Timeout is terminal. On Unix, the runtime starts the skill in a new process
group, sends `SIGTERM` to the group, then sends `SIGKILL` after a short grace
period. Non-Unix runtimes must at least terminate the direct child and report
the platform limitation in tests or docs.

## Output

A zero exit code without timeout or abort maps to a sealed/success status.
Timeout, abort, spawn failure, or non-zero exit maps to failure. Structured JSON
stdout remains author output; graph runners may parse object stdout into step
outputs, but raw stdout and stderr remain visible.

Output ownership is exact. Deterministic and agent runners declare their typed
outputs and packets at the producer. A graph runner declares neither
runner-level `outputs` nor runner-level `artifacts`: the graph receipt proves
the composition, while the terminal output-producing step owns the result and
its packet schema. If a graph needs one reusable result, add an explicit
package/finalize step instead of wrapping the graph trace in a second contract.

## Fixture Gate

`pnpm fixtures:skill-author-runtime:check` runs the same fixture entrypoint
through the TypeScript adapter and Rust runtime. The gate compares only
author-visible behavior: status, stdout/stderr, exit code where relevant,
parsed stdout JSON, cwd relation, input delivery mode, output truncation, and
descendant timeout cleanup.
