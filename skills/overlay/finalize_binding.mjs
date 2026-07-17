import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const root = path.resolve(process.env.RUNX_CWD || process.cwd());
const scriptRoot = path.dirname(fileURLToPath(import.meta.url));
const evidence = requiredRecord(inputs.source_evidence, "source_evidence");
const draft = isRecord(inputs.profile_draft) ? inputs.profile_draft : {};
const source = requiredRecord(evidence.source, "source_evidence.source");
const blockers = [];

let bindingBundle;
try {
  if (evidence.decision !== "ready") throw new Error("source evidence is not ready");
  requiredRecord(draft, "profile_draft");
  if (draft.decision !== "ready") throw new Error("profile draft is not ready");
  const sourcePath = resolveSourcePath(requiredString(source.path, "source.path"));
  const markdown = fs.readFileSync(sourcePath);
  if (`sha256:${sha256(markdown)}` !== source.sha256) throw new Error("upstream SKILL.md changed after inspection");

  const runner = normalizeRunner(requiredRecord(draft.runner, "profile_draft.runner"));
  const harnessCases = normalizeHarness(draft.harness_cases, runner.task);
  const skillName = requiredString(source.name, "source.name");
  const profile = {
    skill: skillName,
    version: "0.1.0",
    runners: {
      [runner.name]: {
        default: true,
        type: "agent-task",
        agent: runner.agent,
        task: runner.task,
        inputs: runner.inputs,
        outputs: runner.outputs,
        runx: {
          scopes: runner.scopes,
          allowed_tools: runner.allowed_tools,
          tags: runner.tags,
          category: runner.category,
          source: {
            upstream: {
              repo: `${evidence.upstream.owner}/${evidence.upstream.repo}`,
              path: evidence.upstream.path,
              commit: evidence.upstream.commit,
              blob_sha: evidence.upstream.blob_sha,
            },
          },
          sandbox: runner.sandbox,
        },
      },
    },
    harness: { cases: harnessCases },
  };
  const profileDocument = `${toYaml(profile)}\n`;
  const proof = proveProfile(markdown, profileDocument, runner.name);
  const bindingPath = requiredString(evidence.binding_path, "binding_path");
  const owner = requiredString(evidence.registry.owner, "registry.owner");
  const skillId = `${owner}/${skillName}`;
  const binding = {
    schema: "runx.registry_binding.v1",
    state: "harness_verified",
    skill: {
      id: skillId,
      name: skillName,
      description: requiredString(source.description, "source.description"),
    },
    upstream: evidence.upstream,
    registry: {
      owner,
      trust_tier: evidence.registry.trust_tier,
      version: evidence.registry.version,
      install_command: `runx add ${skillId}@${evidence.registry.version}`,
      run_command: `runx skill ${skillId}@${evidence.registry.version}`,
      profile_path: `${bindingPath}/X.yaml`,
      materialized_package_is_registry_artifact: true,
    },
    harness: {
      status: "harness_verified",
      case_count: proof.harness.case_count,
      assertion_count: proof.harness.case_count,
      case_names: proof.harness.case_names,
    },
    publication: isRecord(evidence.publication) ? evidence.publication : { status: "not_published" },
    tags: uniqueStrings([...evidence.tags, ...runner.tags]),
  };
  const bindingDocument = `${JSON.stringify(binding, null, 2)}\n`;
  bindingBundle = {
    decision: "ready",
    binding_path: bindingPath,
    source,
    files: [
      fileEntry(`${bindingPath}/binding.json`, bindingDocument),
      fileEntry(`${bindingPath}/X.yaml`, profileDocument),
    ],
    validation: proof,
    rationale: requiredString(draft.rationale, "profile_draft.rationale"),
    blockers: [],
    success_checkpoint: {
      milestone: "binding_bundle_ready",
      description: "Exact native binding files passed profile inspection and isolated harness proof; repository write and publication remain separate.",
    },
  };
} catch (error) {
  blockers.push(boundedMessage(error));
  bindingBundle = {
    decision: "reject",
    binding_path: stringValue(evidence.binding_path) || "",
    source,
    files: [],
    validation: { status: "hold" },
    rationale: stringValue(draft.rationale) || "The binding candidate failed deterministic validation.",
    blockers,
    success_checkpoint: {
      milestone: "binding_blocked",
      description: "No binding files were released.",
    },
  };
}

process.stdout.write(`${JSON.stringify({ binding_bundle: bindingBundle }, null, 2)}\n`);

function normalizeRunner(value) {
  const scopes = uniqueStrings(value.scopes);
  const allowedTools = uniqueStrings(value.allowed_tools);
  for (const tool of allowedTools) {
    if (!/^[a-z][a-z0-9_-]*(?:\.[a-z][a-z0-9_-]*)+$/u.test(tool)) throw new Error(`allowed tool is not a canonical tool ref: ${tool}`);
  }
  return {
    name: packageSegment(value.name, "runner.name"),
    agent: packageSegment(value.agent, "runner.agent"),
    task: packageSegment(value.task, "runner.task"),
    inputs: requiredRecord(value.inputs, "runner.inputs"),
    outputs: requiredRecord(value.outputs, "runner.outputs"),
    scopes,
    allowed_tools: allowedTools,
    sandbox: normalizeSandbox(value.sandbox),
    category: packageSegment(value.category, "runner.category"),
    tags: uniqueStrings(value.tags),
  };
}

function normalizeHarness(value, task) {
  if (!Array.isArray(value) || value.length < 2) throw new Error("profile_draft.harness_cases must contain at least two cases");
  return value.map((entry, index) => {
    const harness = requiredRecord(entry, `harness_cases[${index}]`);
    const caller = requiredRecord(harness.caller, `harness_cases[${index}].caller`);
    const answers = requiredRecord(caller.answers, `harness_cases[${index}].caller.answers`);
    if (!isRecord(answers[`agent_task.${task}.output`])) throw new Error(`harness_cases[${index}] must answer agent_task.${task}.output`);
    return {
      name: packageSegment(harness.name, `harness_cases[${index}].name`),
      inputs: isRecord(harness.inputs) ? harness.inputs : {},
      caller: { answers },
      expect: isRecord(harness.expect) ? harness.expect : { status: "sealed" },
    };
  });
}

function proveProfile(markdown, profileDocument, runnerName) {
  const stage = fs.mkdtempSync(path.join(os.tmpdir(), "runx-binding-profile-"));
  const receipts = fs.mkdtempSync(path.join(os.tmpdir(), "runx-binding-receipts-"));
  try {
    fs.writeFileSync(path.join(stage, "SKILL.md"), markdown);
    fs.writeFileSync(path.join(stage, "X.yaml"), profileDocument);
    const runx = runxBinary();
    const environment = { ...process.env, RUNX_CWD: root };
    const inspect = JSON.parse(execFileSync(runx, ["skill", "inspect", stage, runnerName, "--json"], { encoding: "utf8", env: environment, stdio: ["ignore", "pipe", "pipe"] }));
    const harness = JSON.parse(execFileSync(runx, ["harness", stage, "--receipt-dir", receipts, "--json"], { encoding: "utf8", env: environment, stdio: ["ignore", "pipe", "pipe"] }));
    if (inspect.status !== "ok" || harness.status !== "passed" || harness.assertion_error_count !== 0) throw new Error("native profile inspection or harness proof failed");
    return {
      status: "pass",
      inspect: { status: inspect.status, runner: inspect.runner?.name, readiness: inspect.readiness?.status },
      harness: { status: harness.status, case_count: harness.case_count, case_names: harness.case_names, receipt_count: Array.isArray(harness.receipt_ids) ? harness.receipt_ids.length : 0 },
    };
  } finally {
    fs.rmSync(stage, { recursive: true, force: true });
    fs.rmSync(receipts, { recursive: true, force: true });
  }
}

function runxBinary() {
  const configured = stringValue(process.env.RUNX_DEV_RUST_CLI_BIN);
  if (configured) return configured;
  const local = path.join(root, "crates", "target", "debug", process.platform === "win32" ? "runx.exe" : "runx");
  return fs.existsSync(local) ? local : "runx";
}

function resolveSourcePath(value) {
  if (value.startsWith("skill://")) return path.resolve(scriptRoot, value.slice("skill://".length));
  return path.resolve(root, value);
}

function normalizeSandbox(value) {
  const sandbox = requiredRecord(value, "runner.sandbox");
  const profile = enumValue(sandbox.profile, ["readonly", "default", "networked"], "runner.sandbox.profile");
  const cwdPolicy = enumValue(sandbox.cwd_policy, ["workspace", "skill"], "runner.sandbox.cwd_policy");
  return { profile, cwd_policy: cwdPolicy };
}

function fileEntry(pathValue, contents) {
  return { path: pathValue, contents, sha256: `sha256:${sha256(contents)}` };
}

function toYaml(value, indent = 0) {
  const prefix = " ".repeat(indent);
  if (Array.isArray(value)) {
    if (value.length === 0) return `${prefix}[]`;
    return value.map((entry) => {
      if (isScalar(entry) || isEmptyCollection(entry)) return `${prefix}- ${yamlScalar(entry)}`;
      const lines = toYaml(entry, indent + 2).split("\n");
      lines[0] = `${prefix}- ${lines[0].trimStart()}`;
      return lines.join("\n");
    }).join("\n");
  }
  if (isRecord(value)) {
    const entries = Object.entries(value);
    if (entries.length === 0) return `${prefix}{}`;
    return entries.map(([key, entry]) => {
      const field = yamlKey(key);
      if (isScalar(entry) || isEmptyCollection(entry)) return `${prefix}${field}: ${yamlScalar(entry)}`;
      return `${prefix}${field}:\n${toYaml(entry, indent + 2)}`;
    }).join("\n");
  }
  return `${prefix}${yamlScalar(value)}`;
}

function isScalar(value) {
  return value === null || ["string", "number", "boolean"].includes(typeof value);
}

function isEmptyCollection(value) {
  return (Array.isArray(value) && value.length === 0) || (isRecord(value) && Object.keys(value).length === 0);
}

function yamlKey(value) {
  return /^[A-Za-z_][A-Za-z0-9_.-]*$/u.test(value) ? value : JSON.stringify(value);
}

function yamlScalar(value) {
  if (Array.isArray(value)) return "[]";
  if (isRecord(value)) return "{}";
  if (typeof value !== "string") return JSON.stringify(value);
  if (/^[A-Za-z_][A-Za-z0-9_./:@-]*$/u.test(value) && !new Set(["null", "true", "false", "yes", "no", "on", "off"]).has(value.toLowerCase())) return value;
  return JSON.stringify(value);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function packageSegment(value, field) {
  const parsed = requiredString(value, field);
  if (!/^[a-z0-9][a-z0-9-]*$/u.test(parsed)) throw new Error(`${field} must be a lowercase package segment`);
  return parsed;
}

function enumValue(value, allowed, field) {
  if (!allowed.includes(value)) throw new Error(`${field} must be one of ${allowed.join(", ")}`);
  return value;
}

function uniqueStrings(value) {
  return Array.isArray(value) ? [...new Set(value.map(stringValue).filter(Boolean))].sort() : [];
}

function requiredString(value, field) {
  const parsed = stringValue(value);
  if (!parsed) throw new Error(`${field} must be a non-empty string`);
  return parsed;
}

function stringValue(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isRecord(value) {
  return value && typeof value === "object" && !Array.isArray(value);
}

function requiredRecord(value, field) {
  if (!isRecord(value) || Object.keys(value).length === 0) throw new Error(`${field} must be a non-empty object`);
  return value;
}

function boundedMessage(error) {
  const stderr = error && typeof error === "object" && "stderr" in error ? String(error.stderr || "") : "";
  const stdout = error && typeof error === "object" && "stdout" in error ? String(error.stdout || "") : "";
  const message = stderr || stdout || (error instanceof Error ? error.message : "Binding validation failed");
  return message.replace(/\s+/gu, " ").trim().slice(0, 300);
}
