#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { auditOfficialSkills } from "./lib/skill-operator-value.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binary = path.resolve(
  process.env.RUNX_BIN ?? path.join(root, "crates", "target", "debug", executable("runx")),
);
const outputPath = path.join(root, "docs", "core-skill-trial-results.json");
const providerEvidencePath = path.join(root, "docs", "core-skill-provider-trials.json");
const providerEvidence = JSON.parse(readFileSync(providerEvidencePath, "utf8"));
const decisions = JSON.parse(readFileSync(path.join(root, "docs", "core-skill-review-decisions.json"), "utf8"));
validateProviderEvidence(providerEvidence);
const timeoutMs = integerFlag("--timeout-ms", 120_000);
const write = process.argv.includes("--write");
const check = process.argv.includes("--check");
const json = process.argv.includes("--json");
const strict = process.argv.includes("--strict");

if (process.argv.includes("--managed-agent")) {
  throw new Error("core-skill trials forbid managed-agent execution");
}
if (write && check) throw new Error("choose either --write or --check");
if (!existsSync(binary)) {
  throw new Error(`runx binary is missing at ${binary}; build runx-cli first or set RUNX_BIN`);
}

const skills = auditOfficialSkills(root).filter((skill) => skill.visibility === "public");
const results = skills.map(trialSkill);
const packet = {
  schema: "runx.core_skill_trials.v1",
  execution: {
    managed_agent: false,
    credential_source: "isolated_none",
    cwd: "project_owned_.runx_scratch",
    receipt_signer: "ephemeral_test_key",
  },
  summary: {
    public_skills: results.length,
    locally_proven: results.filter((result) => result.local_trial === "passed").length,
    failed: results.filter((result) => result.local_trial === "failed").length,
    unproven: results.filter((result) => result.local_trial === "unproven").length,
    meets_full_bar: results.filter((result) => result.meets_full_bar).length,
  },
  skills: results,
};
const serialized = `${JSON.stringify(packet, null, 2)}\n`;

if (write) writeFileSync(outputPath, serialized, "utf8");
if (check) {
  if (!existsSync(outputPath) || readFileSync(outputPath, "utf8") !== serialized) {
    throw new Error("core-skill trial results are stale; run with --write");
  }
}
if (json) process.stdout.write(serialized);
else {
  process.stdout.write(
    `trialled ${packet.summary.public_skills} public skills: `
      + `${packet.summary.locally_proven} locally proven, ${packet.summary.failed} failed, `
      + `${packet.summary.unproven} unproven, ${packet.summary.meets_full_bar} meet the full bar\n`,
  );
}
if (packet.summary.failed > 0 || (strict
  && (packet.summary.unproven > 0
    || packet.summary.meets_full_bar !== packet.summary.public_skills))) {
  process.exitCode = 1;
}

function trialSkill(skill) {
  const cases = uniqueProofCases(skill.proof.cases);
  const caseResults = cases.map((entry) => trialFixture(skill.skill, entry));
  if (skill.skill === "skill-lab") caseResults.push(...trialSkillLabWritePaths());
  const localTrial = caseResults.length === 0
    ? "unproven"
    : caseResults.every((entry) => entry.status === "passed")
      ? "passed"
      : "failed";
  const decision = decisions.recommendations?.[skill.skill];
  const archetype = decision?.archetype ?? "unreviewed";
  const providerReadbackRequired = skill.completion === "provider_readback";
  const providerTrial = providerEvidence.skills?.[skill.skill] ?? null;
  const providerReadback = providerReadbackRequired
    ? providerTrial?.status === "passed"
      ? "passed"
      : caseResults.some(
        (entry) => entry.status === "passed" && entry.provider_readback === "live-keyless-read",
      )
        ? "passed_by_live_keyless_fixture"
      : skill.capability_boundaries.includes("http") && localTrial === "passed"
        ? "passed_by_live_http_fixture"
      : "not_proven_by_isolated_fixture"
    : "not_required";
  const operationProofRequired = archetype === "operation" && skill.execution !== "plan";
  const operationProven = caseResults.some(
    (entry) => entry.status === "passed"
      && (entry.proof_type === "operation" || entry.proof_type === "operator_journey"),
  );
  const providerBoundaryProven = providerReadback === "passed"
    || providerReadback === "passed_by_live_keyless_fixture"
    || providerReadback === "passed_by_live_http_fixture";
  const operationBoundaryProven = operationProven || providerBoundaryProven;
  const improvementFindings = operationBoundaryProven
    ? skill.improvements.filter((finding) => finding !== "add standalone operation-boundary proof")
    : skill.improvements;
  return {
    skill: skill.skill,
    path: skill.path,
    archetype,
    decision_status: "pending_review",
    preliminary_route: skill.disposition,
    managed_agent_acts: skill.managed_agent_acts,
    capabilities: skill.capabilities,
    static_findings: skill.issues,
    improvement_findings: improvementFindings,
    local_trial: localTrial,
    operation_proof: operationProven
      ? "passed"
      : providerBoundaryProven
        ? "passed_by_provider_readback"
        : operationProofRequired
          ? "missing"
          : "not_required",
    provider_readback: providerReadback,
    provider_trial: providerTrial,
    meets_full_bar: decision?.action === "keep"
      && skill.issues.length === 0
      && improvementFindings.length === 0
      && localTrial === "passed"
      && (!operationProofRequired || operationBoundaryProven)
      && !providerReadback.startsWith("not_proven"),
    cases: caseResults,
  };
}

function trialSkillLabWritePaths() {
  return [
    trialSkillLabWrite({
      name: "skill-lab-isolated-improve-write",
      runner: "improve",
      inputs: {
        objective: "Add a bounded operator note to the disposable target.",
        target_dir: "skills/trial-target",
        failure_packet: {
          verdict: "needs_update",
          failure_summary: "The disposable target lacks the bounded operator note.",
          improvement_proposals: [{
            target: "skills/trial-target/references/operator-note.md",
            change: "Add the bounded note.",
            rationale: "Proves the improve write path without changing product code.",
            risk: "The disposable target gains one documentation file.",
          }],
          next_harness_checks: ["The target still passes its native harness."],
        },
      },
      answerKey: "agent_task.skill-lab-improve.output",
      changeBundle: {
        decision: "write",
        summary: "Add the bounded operator note.",
        non_goals: ["Do not alter execution behavior."],
        files: [{
          path: "references/operator-note.md",
          contents: "# Operator note\n\nThis file proves the isolated improve write path.\n",
        }],
      },
      verify({ rootDir }) {
        return existsSync(path.join(rootDir, "skills", "trial-target", "references", "operator-note.md"))
          ? null
          : "improve runner did not write the bounded target file";
      },
    }),
    trialSkillLabWrite({
      name: "skill-lab-isolated-harness-write",
      runner: "harness",
      inputs: {
        objective: "Add a second replayable echo case to the disposable target.",
        target_dir: "skills/trial-target",
      },
      answerKey: "agent_task.skill-lab-harness.output",
      changeBundle: {
        decision: "write",
        summary: "Add the second bounded echo case.",
        non_goals: ["Do not alter target behavior."],
        files: [{
          path: "fixtures/echo-second.yaml",
          contents: [
            "name: trial-target-echo-second",
            "kind: skill",
            "target: ..",
            "inputs:",
            "  message: second",
            "expect:",
            "  status: sealed",
            "  output:",
            "    subset:",
            "      message: second",
            "  receipt:",
            "    schema: runx.receipt.v1",
            "",
          ].join("\n"),
        }],
      },
      verify({ rootDir }) {
        return existsSync(path.join(rootDir, "skills", "trial-target", "fixtures", "echo-second.yaml"))
          ? null
          : "harness runner did not write the replayable fixture";
      },
    }),
    trialSkillLabWrite({
      name: "skill-lab-builds-and-runs-execute-package",
      runner: "build",
      inputs: {
        objective: "Build a disposable execute-capable echo package for sandbox proof.",
        target_dir: "skills/generated-execute",
      },
      answerKey: "agent_task.skill-lab-build.output",
      changeBundle: generatedExecuteBundle(),
      verify: verifyGeneratedExecuteTarget,
    }),
  ];
}

function trialSkillLabWrite({ name, runner, inputs, answerKey, changeBundle, verify }) {
  const rootDir = makeTrialRoot("skill-lab-write");
  const receiptDir = path.join(rootDir, "receipts");
  try {
    seedDisposableReadSkill(rootDir);
    const fixturePath = path.join(rootDir, `${runner}.json`);
    writeFileSync(fixturePath, `${JSON.stringify({
      name,
      kind: "skill",
      target: path.join(root, "skills", "skill-lab"),
      runner,
      inputs: { ...inputs, repo_root: rootDir },
      caller: { answers: { [answerKey]: { change_bundle: changeBundle } } },
      expect: { status: "sealed", receipt: { schema: "runx.receipt.v1" } },
    }, null, 2)}\n`);
    const result = runHarness(fixturePath, receiptDir, rootDir, rootDir);
    if (result.error || result.status !== 0) {
      return failedSpecialCase(name, runner, boundedFailure(result.stderr || result.stdout || result.error?.message, rootDir));
    }
    const receipt = parseReceiptResult(result.stdout);
    if (!receipt) return failedSpecialCase(name, runner, "write trial did not return a closed receipt");
    const verificationError = verify({ rootDir, receiptDir });
    if (verificationError) return failedSpecialCase(name, runner, verificationError);
    return {
      name,
      path: "generated:isolated-skill-lab-write",
      runner,
      proof_type: "operation",
      status: "passed",
      receipt,
    };
  } finally {
    rmSync(rootDir, { recursive: true, force: true });
  }
}

function seedDisposableReadSkill(rootDir) {
  const skillDir = path.join(rootDir, "skills", "trial-target");
  mkdirSync(path.join(skillDir, "fixtures"), { recursive: true });
  writeFileSync(path.join(skillDir, "SKILL.md"), [
    "---",
    "name: trial-target",
    "description: Return a bounded echo packet for isolated skill-lab validation.",
    "---",
    "",
    "# Trial Target",
    "",
    "Return one bounded local echo packet.",
    "",
  ].join("\n"));
  writeFileSync(path.join(skillDir, "X.yaml"), [
    "skill: trial-target",
    "version: \"0.1.0\"",
    "catalog:",
    "  kind: skill",
    "  audience: builder",
    "  visibility: public",
    "  role: context",
    "  execution: read",
    "  completion: runtime_receipt",
    "  requires_adapter: false",
    "  approval: none",
    "runners:",
    "  read:",
    "    default: true",
    "    type: cli-tool",
    "    command: node",
    "    args:",
    "      - run.mjs",
    "    inputs:",
    "      message:",
    "        type: string",
    "        required: true",
    "    outputs:",
    "      message: string",
    "",
  ].join("\n"));
  writeFileSync(
    path.join(skillDir, "run.mjs"),
    "const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || \"{}\");\nprocess.stdout.write(`${JSON.stringify({ message: String(inputs.message || \"\") })}\\n`);\n",
  );
  writeFileSync(path.join(skillDir, "fixtures", "echo.yaml"), [
    "name: trial-target-echo",
    "kind: skill",
    "target: ..",
    "inputs:",
    "  message: hello",
    "expect:",
    "  status: sealed",
    "  output:",
    "    subset:",
    "      message: hello",
    "  receipt:",
    "    schema: runx.receipt.v1",
    "",
  ].join("\n"));
}

function generatedExecuteBundle() {
  return {
    decision: "write",
    summary: "Build the disposable execute-capable package.",
    non_goals: ["Do not access a provider or network."],
    files: [
      {
        path: "SKILL.md",
        contents: "---\nname: generated-execute\ndescription: Return one bounded local execution packet.\n---\n\n# Generated Execute\n\nExecute one deterministic local echo.\n",
      },
      {
        path: "X.yaml",
        contents: [
          "skill: generated-execute",
          "version: \"0.1.0\"",
          "catalog:",
          "  kind: skill",
          "  audience: builder",
          "  visibility: public",
          "  role: canonical",
          "  execution: execute",
          "  completion: runtime_receipt",
          "  requires_adapter: false",
          "  approval: none",
          "runners:",
          "  execute:",
          "    default: true",
          "    type: cli-tool",
          "    command: node",
          "    args:",
          "      - run.mjs",
          "    inputs:",
          "      message:",
          "        type: string",
          "        required: true",
          "    outputs:",
          "      message: string",
          "      executed: boolean",
          "",
        ].join("\n"),
      },
      {
        path: "run.mjs",
        contents: "const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || \"{}\");\nprocess.stdout.write(`${JSON.stringify({ message: String(inputs.message || \"\"), executed: true })}\\n`);\n",
      },
    ],
  };
}

function verifyGeneratedExecuteTarget({ rootDir, receiptDir }) {
  const target = path.join(rootDir, "skills", "generated-execute");
  const result = spawnSync(
    binary,
    ["skill", target, "execute", "--input", "message=approved sandbox execution", "--receipt-dir", receiptDir, "--json", "--skip-operator-context"],
    {
      cwd: rootDir,
      env: isolatedEnv(rootDir),
      encoding: "utf8",
      timeout: timeoutMs,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
  if (result.error || result.status !== 0) return boundedFailure(result.stderr || result.stdout || result.error?.message, rootDir);
  try {
    const output = JSON.parse(result.stdout);
    return output.status === "sealed"
      && output.execution?.structured_output?.executed === true
      && output.execution?.structured_output?.message === "approved sandbox execution"
      ? null
      : "generated execute target did not return the expected sealed output";
  } catch (error) {
    return `generated execute target returned invalid JSON: ${error.message}`;
  }
}

function parseReceiptResult(value) {
  try {
    const receipt = JSON.parse(value);
    const evidence = stableReceiptEvidence(receipt);
    return evidence?.disposition === "closed" ? evidence : null;
  } catch {
    return null;
  }
}

function stableReceiptEvidence(receipt) {
  if (receipt?.schema !== "runx.receipt.v1"
    || !/^sha256:[0-9a-f]{64}$/.test(String(receipt.id ?? ""))
    || typeof receipt.seal?.disposition !== "string"
    || typeof receipt.seal?.reason_code !== "string") {
    return null;
  }
  return {
    schema: receipt.schema,
    content_addressed_id: "validated_sha256",
    disposition: receipt.seal.disposition,
    reason_code: receipt.seal.reason_code,
  };
}

function failedSpecialCase(name, runner, reason) {
  return {
    name,
    path: "generated:isolated-skill-lab-write",
    runner,
    proof_type: "operation",
    status: "failed",
    reason,
  };
}

function uniqueProofCases(cases) {
  const seen = new Set();
  return cases.filter((entry) => {
    const key = entry.kind === "inline" ? `inline:${entry.path}` : `fixture:${entry.path}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function trialFixture(skill, fixture) {
  const rootDir = makeTrialRoot("core-skill");
  const receiptDir = path.join(rootDir, "receipts");
  const fixturePath = path.join(root, fixture.path);
  try {
    let result = runHarness(fixturePath, receiptDir, rootDir, rootDir);
    if (result.status !== 0 && String(result.stderr).includes("writable path(s) outside workspace")) {
      const sourceSkillDir = path.join(root, "skills", skill);
      const copiedSkillDir = path.join(rootDir, "workspace", skill);
      mkdirSync(path.dirname(copiedSkillDir), { recursive: true });
      cpSync(sourceSkillDir, copiedSkillDir, { recursive: true });
      const copiedFixture = path.join(copiedSkillDir, path.relative(sourceSkillDir, fixturePath));
      result = runHarness(copiedFixture, receiptDir, copiedSkillDir, rootDir);
    }
    if (result.error) {
      return failedCase(fixture, `process error: ${result.error.message}`);
    }
    if (result.status !== 0) {
      return failedCase(fixture, boundedFailure(result.stderr || result.stdout, rootDir));
    }
    let report;
    try {
      report = JSON.parse(result.stdout);
    } catch (error) {
      return failedCase(fixture, `invalid JSON harness result: ${error.message}`);
    }
    const receipt = stableReceiptEvidence(report);
    if (receipt) {
      return {
        name: fixture.name,
        path: fixture.path,
        runner: fixture.runner,
        proof_type: fixture.proof_type,
        ...(fixture.provider_readback ? { provider_readback: fixture.provider_readback } : {}),
        status: "passed",
        receipt,
      };
    }
    if (report.status === "passed" && Number.isInteger(report.case_count)) {
      return {
        name: fixture.name,
        path: fixture.path,
        runner: fixture.runner,
        proof_type: fixture.proof_type,
        ...(fixture.provider_readback ? { provider_readback: fixture.provider_readback } : {}),
        status: "passed",
        harness: { status: report.status, case_count: report.case_count },
      };
    }
    return failedCase(fixture, "harness did not return a sealed receipt or passing report");
  } finally {
    rmSync(rootDir, { recursive: true, force: true });
  }
}

function runHarness(fixturePath, receiptDir, cwd, rootDir) {
  return spawnSync(
    binary,
    ["harness", fixturePath, "--receipt-dir", receiptDir, "--json"],
    {
      cwd,
      env: isolatedEnv(rootDir),
      encoding: "utf8",
      timeout: timeoutMs,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
}

function failedCase(fixture, reason) {
  return {
    name: fixture.name,
    path: fixture.path,
    runner: fixture.runner,
    proof_type: fixture.proof_type,
    status: "failed",
    reason,
  };
}

function isolatedEnv(rootDir) {
  const binaryDir = path.dirname(binary);
  const inheritedPath = process.env.PATH ?? "";
  const tempDir = path.join(rootDir, "tmp");
  mkdirSync(tempDir, { recursive: true });
  return Object.fromEntries(
    [
      ["PATH", [binaryDir, inheritedPath].filter(Boolean).join(path.delimiter)],
      ["TMPDIR", tempDir],
      ["SSL_CERT_FILE", process.env.SSL_CERT_FILE],
      ["SSL_CERT_DIR", process.env.SSL_CERT_DIR],
      ["HOME", rootDir],
      ["RUNX_HOME", path.join(rootDir, "runx-home")],
      [
        "RUNX_TOOL_ROOTS",
        [process.env.RUNX_TOOL_ROOTS, path.join(root, "tools")]
          .filter(Boolean)
          .join(path.delimiter),
      ],
      ["RUNX_RECEIPT_SIGN_KID", "runx-core-skill-trial-key"],
      [
        "RUNX_RECEIPT_SIGN_ED25519_SEED_BASE64",
        "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=",
      ],
      ["RUNX_RECEIPT_SIGN_ISSUER_TYPE", "hosted"],
      ["NO_COLOR", "1"],
    ].filter(([, value]) => typeof value === "string" && value.length > 0),
  );
}

function makeTrialRoot(label) {
  const scratchRoot = path.join(root, ".runx", "core-skill-trials");
  mkdirSync(scratchRoot, { recursive: true });
  return mkdtempSync(path.join(scratchRoot, `${label}-`));
}

function boundedFailure(value, rootDir) {
  return String(value || "trial failed without output")
    .replaceAll(rootDir, "<trial-dir>")
    .replaceAll(root, "<repo>")
    .trim()
    .slice(0, 2_000);
}

function integerFlag(name, fallback) {
  const prefix = `${name}=`;
  const value = process.argv.find((entry) => entry.startsWith(prefix));
  if (!value) return fallback;
  const parsed = Number(value.slice(prefix.length));
  if (!Number.isInteger(parsed) || parsed < 1) throw new Error(`${name} expects a positive integer`);
  return parsed;
}

function executable(name) {
  return process.platform === "win32" ? `${name}.exe` : name;
}

function validateProviderEvidence(value) {
  if (value?.schema !== "runx.core_skill_provider_trials.v1" || !value.skills) {
    throw new Error("invalid core-skill provider trial evidence");
  }
  const serialized = JSON.stringify(value);
  if (/api[_-]?key|authorization|bearer|credential_material_ref|secret|token/i.test(serialized)) {
    throw new Error("provider trial evidence contains a forbidden credential field");
  }
  for (const [skill, evidence] of Object.entries(value.skills)) {
    if (evidence.status !== "passed" || evidence.managed_agent !== false || evidence.mutation !== false) {
      throw new Error(`provider evidence for ${skill} is not a passed, no-agent, read-only trial`);
    }
    if (!/^sha256:[a-f0-9]{64}$/u.test(evidence.receipt_id ?? "")) {
      throw new Error(`provider evidence for ${skill} has no sealed receipt id`);
    }
  }
}
