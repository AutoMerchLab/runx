import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  validateRunnerManifestYamlBatch,
  validateSkillMarkdownBatch,
} from "./lib/native-parser.mjs";

const workspaceRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const fixtureRoot = path.join(workspaceRoot, "fixtures", "runtime", "skills");
const check = process.argv.includes("--check");
const generatedAt = "2026-05-18T00:00:00Z";
const skillNames = ["issue-intake"] as const;
const retiredReceiptFields = [
  "kind",
  retiredExecutionShape("skill"),
  retiredExecutionShape("graph"),
  "skill_name",
  "source_type",
  "graph_name",
  "owner",
];

interface ParsedRunnerManifest {
  readonly skill?: string;
  readonly runners: Readonly<Record<string, {
    readonly name: string;
    readonly default: boolean;
    readonly source: {
      readonly type: string;
      readonly task?: string;
      readonly graph?: { readonly steps: readonly unknown[] };
    };
  }>>;
  readonly harness?: {
    readonly cases: readonly Record<string, unknown>[];
  };
}

process.chdir(workspaceRoot);

const packages = await Promise.all(skillNames.map(async (skillName) => {
  const skillDir = path.join(workspaceRoot, "skills", skillName);
  return {
    skillName,
    skillMarkdown: await readFile(path.join(skillDir, "SKILL.md"), "utf8"),
    profileSource: await readFile(path.join(skillDir, "X.yaml"), "utf8"),
  };
}));
const skills = validateSkillMarkdownBatch(packages.map((entry) => entry.skillMarkdown));
const profiles = validateRunnerManifestYamlBatch(
  packages.map((entry) => entry.profileSource),
) as ParsedRunnerManifest[];
for (const [index, entry] of packages.entries()) {
  const profile = profiles[index];
  if (!profile) throw new Error(`native parser omitted skills/${entry.skillName}/X.yaml`);
  await generateSkillFixtures(
    entry.skillName,
    entry.skillMarkdown,
    entry.profileSource,
    profile,
    (skills[index] as { name: string }).name,
  );
}

console.log(`${check ? "checked" : "generated"} Rust product skill fixtures`);

function retiredExecutionShape(prefix: string): string {
  return `${prefix}_${"execution"}`;
}

async function generateSkillFixtures(
  skillName: typeof skillNames[number],
  skillMarkdown: string,
  profileSource: string,
  profile: ParsedRunnerManifest,
  declaredSkillName: string,
): Promise<void> {
  const profilePath = path.join(workspaceRoot, "skills", skillName, "X.yaml");
  if (declaredSkillName !== skillName || profile.skill !== skillName) {
    throw new Error(`${skillName}: product skill name drifted from SKILL.md/X.yaml`);
  }
  const execution = fixtureExecution(profile, profilePath);
  const cases = harnessCases(profile, profilePath);
  const targetDir = path.join(fixtureRoot, skillName);
  if (!check) {
    await rm(targetDir, { recursive: true, force: true });
  }
  await mkdir(path.join(targetDir, "cases"), { recursive: true });

  await writeOrCheck(path.join(targetDir, "metadata.json"), `${JSON.stringify({
    schema: "runx.runtime.skill_fixture.v1",
    generated_at: generatedAt,
    source: {
      skill: path.posix.join("skills", skillName, "SKILL.md"),
      profile: path.posix.join("skills", skillName, "X.yaml"),
    },
    skill_name: skillName,
    manifest_hash: `sha256:${sha256(`${skillMarkdown}\n${profileSource}`)}`,
    harness_schema: "runx.receipt.v1",
    case_names: cases.map((entry) => String(entry.name)),
  }, null, 2)}\n`);

  for (const entry of cases) {
    const normalizedEntry = withIntakeDecision(entry);
    const fixture = intakeFixture(normalizedEntry, skillName, execution);
    assertNoRetiredReceiptFields(fixture, `${skillName}.${normalizedEntry.name}`);
    await writeOrCheck(
      path.join(targetDir, "cases", `${normalizedEntry.name}.yaml`),
      yaml(fixture),
    );
  }

  if (check) {
    await assertNoStaleCases(targetDir, cases);
  }
}

function intakeFixture(
  entry: Record<string, unknown>,
  skillName: string,
  execution: { readonly kind: "agent_task"; readonly runner: string },
): Record<string, unknown> {
  return {
    name: entry.name,
    kind: execution.kind,
    runner: execution.runner,
    inputs: entry.inputs ?? {},
    caller: entry.caller ?? {},
    expect: canonicalExpectation(entry, {
      status: "sealed",
      state: "sealed",
      disposition: "closed",
    }),
    metadata: {
      product_skill: skillName,
      source_case: entry.name,
      runner_kind: "agent_task",
    },
  };
}

function fixtureExecution(
  profile: ParsedRunnerManifest,
  sourcePath: string,
): { readonly kind: "agent_task"; readonly runner: string } {
  const defaults = Object.values(profile.runners).filter((runner) => runner.default);
  const runner = defaults.length === 1
    ? defaults[0]
    : Object.values(profile.runners).length === 1
      ? Object.values(profile.runners)[0]
      : undefined;
  if (!runner) {
    throw new Error(`${sourcePath}: fixture generation requires one parser-selected default runner`);
  }
  const graphSteps = runner.source.graph?.steps;
  if (runner.source.type === "graph" && !graphSteps) {
    throw new Error(`${sourcePath}#${runner.name}: parser-owned graph omitted typed steps`);
  }
  if (runner.source.type !== "agent-task" || !runner.source.task) {
    throw new Error(
      `${sourcePath}#${runner.name}: generated receipt fixture requires an agent-task source`,
    );
  }
  return { kind: "agent_task", runner: runner.source.task };
}

function withIntakeDecision(entry: Record<string, unknown>): Record<string, unknown> {
  const clone = JSON.parse(JSON.stringify(entry)) as Record<string, unknown>;
  const caller = record(clone.caller, "caller");
  const answers = record(caller?.answers, "caller.answers");
  const output = record(answers?.["agent_task.issue-intake.output"], "caller.answers.agent_task.issue-intake.output");
  if (!output || output.decision) {
    return clone;
  }
  const report = record(output.intake_report, "intake_report") ?? {};
  output.decision = {
    schema: "runx.decision.v1",
    decision_id: `dec_${clone.name}`,
    choice: decisionChoice(report.action_decision),
    summary: report.rationale ?? report.summary ?? "issue-intake selected the next governed boundary",
    recommended_lane: report.recommended_lane ?? "manual-review",
  };
  return clone;
}

function decisionChoice(value: unknown): string {
  switch (value) {
    case "proceed_to_build":
    case "proceed_to_plan":
      return "open";
    case "request_review":
      return "defer";
    case "stop":
      return "decline";
    default:
      return "monitor";
  }
}

function canonicalExpectation(
  entry: Record<string, unknown>,
  receipt: {
    status: string;
    state: string;
    disposition: string;
    childReceiptCount?: number;
  },
): Record<string, unknown> {
  const status = record(entry.expect, "expect")?.status ?? receipt.status;
  const receiptExpectation: Record<string, unknown> = {
    schema: "runx.receipt.v1",
    state: receipt.state,
    disposition: receipt.disposition,
  };
  if (receipt.childReceiptCount !== undefined) {
    receiptExpectation.child_receipt_count = receipt.childReceiptCount;
  }
  return {
    status,
    receipt: receiptExpectation,
  };
}

function harnessCases(profile: ParsedRunnerManifest, sourcePath: string): Record<string, unknown>[] {
  const cases = profile.harness?.cases;
  if (!cases) {
    throw new Error(`${sourcePath}: harness.cases must be an array`);
  }
  return cases.map((entry, index) => {
    const value = record(entry, `${sourcePath}.harness.cases[${index}]`);
    if (!value || typeof value.name !== "string" || value.name.length === 0) {
      throw new Error(`${sourcePath}: harness.cases[${index}].name is required`);
    }
    return value;
  });
}

function assertNoRetiredReceiptFields(value: unknown, label: string): void {
  const findings: string[] = [];
  visit(value, [], (pathSegments, key) => {
    if (retiredReceiptFields.includes(key) && pathSegments.includes("receipt")) {
      findings.push(`${label}:${pathSegments.concat(key).join(".")}`);
    }
  });
  if (findings.length > 0) {
    throw new Error(`retired receipt expectation fields found:\n${findings.join("\n")}`);
  }
}

async function assertNoStaleCases(
  targetDir: string,
  cases: Record<string, unknown>[],
): Promise<void> {
  const expected = new Set(cases.map((entry) => `${entry.name}.yaml`));
  const casesDir = path.join(targetDir, "cases");
  let entries: string[];
  try {
    entries = await readdir(casesDir);
  } catch {
    entries = [];
  }
  const stale = entries.filter((entry) => !expected.has(entry));
  if (stale.length > 0) {
    throw new Error(`${casesDir}: stale generated fixture(s): ${stale.join(", ")}`);
  }
}

async function writeOrCheck(filePath: string, contents: string): Promise<void> {
  if (check) {
    const current = await readFile(filePath, "utf8").catch(() => undefined);
    if (current !== contents) {
      throw new Error(`${path.relative(workspaceRoot, filePath)} is stale; run pnpm tsx scripts/generate-rust-skill-fixtures.ts`);
    }
    return;
  }
  await mkdir(path.dirname(filePath), { recursive: true });
  await writeFile(filePath, contents);
}

function yaml(value: unknown, indent = 0): string {
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return `${" ".repeat(indent)}[]\n`;
    }
    return value.map((entry) => isScalar(entry)
      ? `${" ".repeat(indent)}- ${scalar(entry)}\n`
      : `${" ".repeat(indent)}-\n${yaml(entry, indent + 2)}`).join("");
  }
  const object = record(value, "yaml") ?? {};
  if (Object.keys(object).length === 0) {
    return `${" ".repeat(indent)}{}\n`;
  }
  return Object.entries(object).map(([key, entry]) => {
    if (entry === undefined) {
      return "";
    }
    if (isScalar(entry)) {
      return `${" ".repeat(indent)}${key}: ${scalar(entry)}\n`;
    }
    return `${" ".repeat(indent)}${key}:\n${yaml(entry, indent + 2)}`;
  }).join("");
}

function scalar(value: unknown): string {
  if (value === null) {
    return "null";
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  const stringValue = String(value);
  return JSON.stringify(stringValue);
}

function isScalar(value: unknown): boolean {
  return value === null || ["string", "number", "boolean"].includes(typeof value);
}

function record(value: unknown, _field: string): Record<string, unknown> | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

function visit(
  value: unknown,
  pathSegments: string[],
  onKey: (pathSegments: string[], key: string) => void,
): void {
  if (Array.isArray(value)) {
    value.forEach((entry, index) => visit(entry, pathSegments.concat(String(index)), onKey));
    return;
  }
  const object = record(value, "visit");
  if (!object) {
    return;
  }
  for (const [key, entry] of Object.entries(object)) {
    onKey(pathSegments, key);
    visit(entry, pathSegments.concat(key), onKey);
  }
}

function sha256(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}
