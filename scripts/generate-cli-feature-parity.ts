import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

interface NativeCommandSpec {
  readonly name: string;
  readonly topLevelUsage: readonly string[];
  readonly usage: readonly string[];
  readonly notes: readonly string[];
  readonly options: readonly string[];
}

interface NativeCommandCatalog {
  readonly schema: "runx.cli_command_catalog.v1";
  readonly root: NativeCommandSpec;
  readonly commands: readonly NativeCommandSpec[];
}

interface CommandAnnotation {
  readonly sideEffect: "none" | "filesystem" | "local-runtime" | "adapter" | "external-stub";
  readonly surfaces: readonly string[];
  readonly cases: readonly string[];
  readonly jsonOutput?: "schema-exact" | "none";
}

interface CommandMatrixEntry extends NativeCommandSpec {
  readonly id: string;
  readonly exitCodes: readonly number[];
  readonly parity: {
    readonly humanOutput: "semantic" | "none";
    readonly jsonOutput: "schema-exact" | "none";
    readonly receipt: "schema-exact" | "none";
    readonly sideEffect: "none" | "filesystem" | "local-runtime" | "adapter" | "external-stub";
    readonly surfaces: readonly string[];
  };
  readonly cases: readonly string[];
}

interface RuntimeSurface {
  readonly id: string;
  readonly owner: string;
  readonly parityClass: "schema-exact" | "semantic" | "fixture-backed" | "stubbed";
  readonly coveredBy: readonly string[];
  readonly notes: string;
}

interface OracleCase {
  readonly id: string;
  readonly commandId: string;
  readonly mode: "execute" | "validate";
  readonly argv?: readonly string[];
  readonly expectedExitCode?: number;
  readonly expectJson?: boolean;
  readonly expect?: {
    readonly pendingRuns: number;
    readonly firstPendingRunId: string;
    readonly firstPendingRunStatus: string;
  };
  readonly stdoutIncludes?: readonly string[];
  readonly stderrIncludes?: readonly string[];
  readonly proves: readonly string[];
}

const check = process.argv.includes("--check");
const root = resolve(".");
const fixturesDir = join(root, "fixtures/cli-parity");
const casesDir = join(fixturesDir, "cases");

const exitCodes = [0, 1, 2, 3, 64] as const;

const commandAnnotations: Readonly<Record<string, CommandAnnotation>> = {
  "cli.help": annotation("none", ["cli-presentation"], ["help.top-level", "usage.unsupported"]),
  new: annotation("local-runtime", ["skill-authoring", "graph-runtime", "caller-mediated-resolution", "receipts", "cli-presentation"], ["new.validate"]),
  init: annotation("filesystem", ["workspace-init", "official-skills"], ["init.validate"]),
  verify: annotation("none", ["receipts", "cli-presentation"], ["verify.validate"]),
  history: annotation("none", ["history", "receipts"], ["history.execute"]),
  resume: annotation("local-runtime", ["caller-mediated-resolution", "graph-runtime", "receipts", "cli-presentation"], ["resume.validate"]),
  list: annotation("none", ["list", "tool-catalog"], ["list.tools.execute"]),
  login: annotation("filesystem", ["config", "cli-presentation"], ["login.validate"]),
  connect: annotation("external-stub", ["public-api", "connect", "cli-presentation"], ["connect.execute"]),
  config: annotation("filesystem", ["config", "cli-presentation"], ["config.set.validate", "config.get.validate", "config.list.execute"]),
  credential: annotation("filesystem", ["config", "skill-resolution", "cli-presentation"], ["credential.validate"]),
  policy: annotation("none", ["policy", "cli-presentation"], ["policy.inspect.validate", "policy.lint.validate"]),
  publish: annotation("external-stub", ["receipts", "cli-presentation"], ["publish.validate"]),
  kernel: annotation("local-runtime", ["graph-runtime", "cli-presentation"], ["kernel.validate"]),
  payment: annotation("local-runtime", ["authority", "cli-presentation"], ["payment.validate"]),
  parser: annotation("local-runtime", ["parser", "cli-presentation"], ["parser.validate"]),
  doctor: annotation("filesystem", ["doctor", "cli-presentation"], ["doctor.validate"]),
  data: annotation("filesystem", ["data", "cli-presentation"], ["data.validate"]),
  dev: annotation("local-runtime", ["dev", "harness", "receipts"], ["dev.validate"]),
  export: annotation("filesystem", ["skill-export", "cli-presentation"], ["export.validate"]),
  mcp: annotation("adapter", ["mcp", "adapter-mcp"], ["mcp.serve.validate"], "none"),
  skill: annotation("local-runtime", ["skill-resolution", "graph-runtime", "receipts", "sandbox", "authority", "caller-mediated-resolution", "adapter-cli-tool", "adapter-agent", "cli-presentation"], ["skill.run.validate", "skill.inspect.validate"]),
  add: annotation("external-stub", ["registry", "cli-presentation"], ["add.validate"]),
  harness: annotation("local-runtime", ["harness", "receipts", "sandbox"], ["harness.execute"]),
  tool: annotation("external-stub", ["tool-catalog", "extension-sdk"], ["tool.build.validate", "tool.search.validate", "tool.inspect.validate"]),
  registry: annotation("external-stub", ["registry", "cli-presentation"], ["registry.validate"]),
};

const commands = bindNativeCommands(readNativeCommandCatalog(), commandAnnotations);

const surfaces: readonly RuntimeSurface[] = [
  surface("cli-presentation", "runx-cli", "semantic", ["cli.help", "config"], "Human output is normalized semantically; JSON output stays schema-exact."),
  surface("skill-resolution", "runx-cli + runx-runtime + runx-core", "fixture-backed", ["skill", "registry"], "Covers local paths, registry refs, and official skill resolution."),
  surface("graph-runtime", "runx-runtime", "fixture-backed", ["skill", "harness", "kernel"], "Covers graph execution, branching, caller handoffs, receipts, and the deterministic decision kernel."),
  surface("receipts", "runx-receipts + runx-runtime + runx-cli", "schema-exact", ["skill", "harness", "history", "verify"], "Receipt JSON and signature metadata are schema-exact parity surfaces."),
  surface("ledger", "runx-runtime", "schema-exact", ["history"], "Append-only run state and continuation history must survive cutover."),
  surface("sandbox", "runx-core/policy + runx-runtime", "schema-exact", ["skill", "harness"], "Declared and enforced sandbox metadata must remain distinct."),
  surface("harness", "runx-runtime harness via runx-cli", "fixture-backed", ["harness", "dev"], "Harness replay mode proves deterministic fixture execution and sealed receipt checks."),
  surface("history", "runx-cli + runx-runtime", "semantic", ["history"], "Search/filter behavior is command-level parity with normalized output."),
  surface("registry", "runx-cli + runx-runtime registry", "fixture-backed", ["registry"], "Local and hosted registry envelopes are exercised through native registry commands."),
  surface("tool-catalog", "runx-runtime tool catalogs", "fixture-backed", ["tool", "list"], "Catalog discovery, dispatch, and local tool builds use the canonical native or manifest-owned path."),
  surface("mcp", "runx-runtime adapters/mcp", "stubbed", ["mcp"], "Protocol behavior uses local servers and deterministic clients."),
  surface("adapter-cli-tool", "runx-runtime cli-tool adapter", "fixture-backed", ["skill"], "Process invocation, env, cwd, and sandbox metadata are parity-critical."),
  surface("adapter-mcp", "runx-runtime MCP adapter", "stubbed", ["mcp"], "MCP transport and tool results use local protocol fixtures."),
  surface("adapter-agent", "runx-runtime external agent adapter", "stubbed", ["skill", "dev"], "Managed agent calls are represented by local stubs, not live providers."),
  surface("config", "runx-cli", "schema-exact", ["config", "credential"], "RUNX_HOME, encrypted local profiles, and config file behavior are part of CLI parity."),
  surface("public-api", "runx-cli + runx-runtime", "stubbed", ["login", "connect", "publish"], "Public API identity and HTTP transport are resolved once and exercised against deterministic local servers."),
  surface("connect", "runx-cli + runx cloud", "stubbed", ["connect"], "The native CLI owns provider-neutral grant lifecycle; governed skills and native provider tools own bounded provider operations."),
  surface("doctor", "runx-cli + runx-runtime doctor", "semantic", ["doctor"], "Diagnostics can add ids, but the documented command surface must not disappear."),
  surface("data", "runx-runtime + runx-cli", "fixture-backed", ["data"], "Offline data-store migration is bounded, backup-first, idempotent, and independently read back."),
  surface("dev", "runx-cli", "fixture-backed", ["dev"], "Development lanes run deterministic or recorded harness fixtures."),
  surface("skill-export", "runx-cli + runx-runtime", "semantic", ["export"], "Host-agent shims are generated from validated skill packages and delegate back to governed runx skill execution."),
  surface("parser", "runx-parser via runx-cli", "schema-exact", ["parser"], "Native parser evaluation output stays schema-exact."),
  surface("authority", "runx-core/policy", "schema-exact", ["skill", "payment"], "Grant, scope, and authority-kind policy remains machine-checkable without OSS brokerage."),
  surface("policy", "runx-core/policy", "schema-exact", ["policy"], "Policy inspection and linting stay machine-checkable before mutation gates run."),
  surface("caller-mediated-resolution", "runx-runtime", "fixture-backed", ["skill"], "Required input, approvals, and agent work keep the same continuation contract."),
  surface("skill-authoring", "runx-runtime + skill-lab", "fixture-backed", ["new", "skill"], "Skill creation uses one digest-bound inspect, plan, bind, validate, harness, and transactional apply lane."),
  surface("workspace-init", "runx-runtime", "semantic", ["init"], "Deterministic project and global workspace initialization remains separate from skill authoring."),
  surface("official-skills", "runx-cli", "schema-exact", ["init"], "Prefetch and lockfile behavior stays fixture-backed."),
  surface("list", "runx-cli", "semantic", ["list"], "Inventory output for tools, skills, graphs, packets, and overlays stays represented."),
  surface("extension-sdk", "packages/extension-sdk", "schema-exact", ["tool"], "External process extension output and manifest validation remain schema-exact."),
];

const casesExecutedById = new Set([
  "help.top-level",
  "usage.unsupported",
  "config.list.execute",
  "harness.execute",
  "history.execute",
  "list.tools.execute",
  "connect.execute",
]);

const cases: readonly OracleCase[] = [
  execute("help.top-level", "cli.help", ["--help"], 0, false, ["Usage:", "runx skill", "runx harness"], []),
  execute("usage.unsupported", "cli.help", ["not-a-command"], 64, false, [], ["unknown command not-a-command"]),
  execute("config.list.execute", "config", ["config", "list", "--json"], 0, true, [], []),
  execute("harness.execute", "harness", ["harness", "fixtures/cli-parity/harness/echo-skill.yaml", "--json"], 0, true, [], []),
  {
    id: "history.execute",
    commandId: "history",
    mode: "execute",
    argv: ["history", "--receipt-dir", "$FIXTURE_RECEIPTS", "--json"],
    expectedExitCode: 0,
    expectJson: true,
    expect: {
      pendingRuns: 1,
      firstPendingRunId: "gx_needs_agent_oracle",
      firstPendingRunStatus: "paused",
    },
    stdoutIncludes: ["\"pendingRuns\"", "\"gx_needs_agent_oracle\"", "\"selectedRunner\": \"agent-task\""],
    stderrIncludes: [],
    proves: ["history", "ledger", "receipts", "cli-presentation"],
  },
  execute("list.tools.execute", "list", ["list", "tools", "--json"], 0, true, [], []),
  execute("connect.execute", "connect", ["connect", "list", "--api-base-url", "http://127.0.0.1:9", "--token", "rxk_fixture", "--json"], 1, true, ["not publicly routable"], []),
  ...commands.flatMap((entry) => entry.cases
    .filter((caseId) => !casesExecutedById.has(caseId))
    .map((caseId) => validate(caseId, entry.id, entry.parity.surfaces))),
];

const files = new Map<string, string>([
  [join(fixturesDir, "README.md"), readme()],
  [join(fixturesDir, "commands.json"), stableJson({ schema: "runx.cli_feature_parity_matrix.v1", sourceOfTruth: "runx --help --json", exitCodes, commands })],
  [join(fixturesDir, "runtime-surfaces.json"), stableJson({ schema: "runx.cli_runtime_surfaces.v1", surfaces })],
  [join(casesDir, "oracle.json"), stableJson({ schema: "runx.cli_parity_oracle_cases.v1", cases })],
]);

if (check) {
  checkFiles();
} else {
  writeFiles();
}

function annotation(
  sideEffect: CommandAnnotation["sideEffect"],
  surfaces: readonly string[],
  casesForCommand: readonly string[],
  jsonOutput: CommandAnnotation["jsonOutput"] = "schema-exact",
): CommandAnnotation {
  return { sideEffect, surfaces, cases: casesForCommand, jsonOutput };
}

function readNativeCommandCatalog(): NativeCommandCatalog {
  const defaultBinary = join(
    root,
    "crates",
    "target",
    "debug",
    process.platform === "win32" ? "runx.exe" : "runx",
  );
  const runx = process.env.RUNX_RUST_CLI_BIN ?? defaultBinary;
  if (!existsSync(runx)) {
    throw new Error(
      `native Runx CLI is required to project command parity; build runx-cli or set RUNX_RUST_CLI_BIN (looked for ${runx})`,
    );
  }

  const result = spawnSync(runx, ["--help", "--json"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `native command catalog failed with exit ${result.status ?? "signal"}: ${(result.stderr ?? "").trim()}`,
    );
  }

  const parsed: unknown = JSON.parse(result.stdout);
  if (!isNativeCommandCatalog(parsed)) {
    throw new Error("runx --help --json returned an invalid runx.cli_command_catalog.v1 payload");
  }
  return parsed;
}

function bindNativeCommands(
  catalog: NativeCommandCatalog,
  annotations: Readonly<Record<string, CommandAnnotation>>,
): readonly CommandMatrixEntry[] {
  const nativeCommands = [catalog.root, ...catalog.commands];
  const nativeNames = new Set<string>();
  for (const command of nativeCommands) {
    if (!nativeNames.add(command.name)) {
      throw new Error(`native command catalog contains duplicate command '${command.name}'`);
    }
  }

  const missingAnnotations = nativeCommands
    .map((command) => command.name)
    .filter((name) => annotations[name] === undefined);
  const unknownAnnotations = Object.keys(annotations).filter((name) => !nativeNames.has(name));
  if (missingAnnotations.length > 0 || unknownAnnotations.length > 0) {
    throw new Error([
      "native command catalog and parity annotations disagree",
      `Missing annotations: ${missingAnnotations.join(", ") || "none"}`,
      `Unknown annotations: ${unknownAnnotations.join(", ") || "none"}`,
    ].join("\n"));
  }

  return nativeCommands.map((command) => {
    const metadata = annotations[command.name];
    if (!metadata) {
      throw new Error(`missing parity annotation for '${command.name}'`);
    }
    return {
      ...command,
      id: command.name,
      exitCodes,
      parity: {
        humanOutput: "semantic",
        jsonOutput: metadata.jsonOutput ?? "schema-exact",
        receipt: metadata.surfaces.includes("receipts") ? "schema-exact" : "none",
        sideEffect: metadata.sideEffect,
        surfaces: metadata.surfaces,
      },
      cases: metadata.cases,
    };
  });
}

function isNativeCommandCatalog(value: unknown): value is NativeCommandCatalog {
  if (!isObject(value) || value.schema !== "runx.cli_command_catalog.v1") {
    return false;
  }
  return isNativeCommandSpec(value.root)
    && Array.isArray(value.commands)
    && value.commands.every(isNativeCommandSpec);
}

function isNativeCommandSpec(value: unknown): value is NativeCommandSpec {
  return isObject(value)
    && typeof value.name === "string"
    && isStringArray(value.topLevelUsage)
    && isStringArray(value.usage)
    && isStringArray(value.notes)
    && isStringArray(value.options);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === "string");
}

function surface(
  id: string,
  owner: string,
  parityClass: RuntimeSurface["parityClass"],
  coveredBy: readonly string[],
  notes: string,
): RuntimeSurface {
  return { id, owner, parityClass, coveredBy, notes };
}

function execute(
  id: string,
  commandId: string,
  argv: readonly string[],
  expectedExitCode: number,
  expectJson: boolean,
  stdoutIncludes: readonly string[],
  stderrIncludes: readonly string[],
): OracleCase {
  return {
    id,
    commandId,
    mode: "execute",
    argv,
    expectedExitCode,
    expectJson,
    stdoutIncludes,
    stderrIncludes,
    proves: commands.find((entry) => entry.id === commandId)?.parity.surfaces ?? [],
  };
}

function validate(id: string, commandId: string, proves: readonly string[]): OracleCase {
  return { id, commandId, mode: "validate", proves };
}

function readme(): string {
  return `# CLI Feature Parity Matrix

This directory captures the canonical native Rust CLI/runtime surface. The
matrix projects command syntax directly from \`runx --help --json\`.
\`scripts/generate-cli-feature-parity.ts\` adds only test, effect, and runtime
surface annotations keyed by native command name.

Required exit-code coverage: \`"exitCodes": [0, 1, 2, 3, 64]\`.

## Files

- \`commands.json\`: native usage/options plus exit-code, output, receipt, and
  side-effect coverage.
- \`runtime-surfaces.json\`: non-help runtime surfaces that must not disappear
  during a Rust rebuild.
- \`cases/oracle.json\`: executable or validation-only oracle cases.

## Parity Rules

- JSON output and receipt behavior are schema-exact.
- Human output is semantic and may be normalized for timestamps, paths,
  receipt ids, and platform-specific wording.
- Live providers are replaced by deterministic mocks, fixtures, or local
  protocol servers.
- Native CLI candidates must pass this matrix before packaging.
`;
}

function checkFiles(): void {
  const stale = [...files.entries()]
    .filter(([path, contents]) => !existsSync(path) || readFileSync(path, "utf8") !== contents)
    .map(([path]) => path);
  if (stale.length > 0) {
    throw new Error(`CLI parity fixtures are stale; run this script without --check:\n${stale.join("\n")}`);
  }
  const caseFiles = readdirSync(casesDir).filter((name) => name.endsWith(".json"));
  if (!caseFiles.includes("oracle.json")) {
    throw new Error("fixtures/cli-parity/cases/oracle.json is missing");
  }
}

function writeFiles(): void {
  for (const [path, contents] of files) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, contents);
  }
}

function stableJson(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}
