#!/usr/bin/env node

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { auditOfficialSkills, semanticCapabilityFindings } from "./lib/skill-operator-value.mjs";

if (process.argv.includes("--self-test")) {
  runSelfTests();
  process.exit(0);
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const skills = auditOfficialSkills(root).filter((skill) => skill.visibility === "public");
const decisions = JSON.parse(readFileSync(path.join(root, "docs", "core-skill-review-decisions.json"), "utf8"));
const findings = admissionFindings(skills, decisions);

if (findings.length > 0) {
  for (const finding of findings) console.error(finding);
  process.exit(1);
}
console.log("archetype-aware public skill capability gate passed");

function admissionFindings(skills, decisions) {
  return skills.flatMap((skill) => {
    const findings = skill.issues.map((issue) => `${skill.path}: ${issue}`);
    const decision = decisions.recommendations?.[skill.skill];
    if (!decision?.archetype) findings.push(`${skill.path}: missing product archetype review`);
    if (decision?.action === "keep" && skill.proof.semantic_cases === 0) {
      findings.push(`${skill.path}: a kept public skill needs at least one semantic output oracle`);
    }
    return findings;
  });
}

function runSelfTests() {
  const canonical = {
    kind: "skill",
    audience: "operator",
    visibility: "public",
    role: "canonical",
    execution: "execute",
    completion: "runtime_receipt",
    requires_adapter: false,
    approval: "none",
  };
  const direct = profile(canonical, { type: "cli-tool", command: "example" });
  assertNoFindings(direct, "deterministic operation");

  const directTool = profile(
    { ...canonical, requires_adapter: true },
    { type: "graph", graph: { steps: [{ id: "read", tool: "example.read" }] } },
  );
  assertNoFindings(directTool, "direct graph tool");

  const agentOnly = profile(canonical, {
    type: "agent-task",
    task: "write-something",
    artifacts: { wrap_as: "draft", packet: "example.draft.v1" },
  });
  assertNoFindings(agentOnly, "agent-authored artifact");

  const namedArtifact = profile(canonical, {
    type: "agent-task",
    task: "write-something",
    artifacts: { named_emits: { draft: "draft" }, packets: { draft: "example.draft.v1" } },
  });
  assertNoFindings(namedArtifact, "agent-authored named artifact");

  const graphStepArtifact = profile(canonical, {
    type: "graph",
    graph: {
      steps: [{
        id: "draft",
        run: { type: "agent-task", task: "write-something" },
        artifacts: { named_emits: { draft: "draft" } },
      }],
    },
  });
  assertNoFindings(graphStepArtifact, "graph-step agent artifact");

  const context = profile(
    { ...canonical, role: "context" },
    {
      type: "agent-task",
      task: "build-context",
      artifacts: { wrap_as: "context", packet: "example.context.v1" },
    },
  );
  assertNoFindings(context, "public-context");

  const agentWithoutArtifact = profile(canonical, { type: "agent-task", task: "write-something" });
  assertFindings(agentWithoutArtifact, "agent-without-artifact", ["no declared artifact packet"]);

  const adapterClaim = profile(
    { ...canonical, requires_adapter: true },
    { type: "cli-tool", command: "example" },
  );
  assertFindings(adapterClaim, "false-adapter-claim", ["no reachable adapter"]);

  const multiRunnerAdapter = {
    skill: "example",
    catalog: { ...canonical, requires_adapter: true },
    runners: {
      plan: {
        default: true,
        type: "agent-task",
        task: "plan",
        artifacts: { wrap_as: "plan", packet: "example.plan.v1" },
      },
      execute: {
        type: "graph",
        graph: { steps: [{ id: "act", tool: "example.execute" }] },
      },
    },
  };
  assertNoFindings(multiRunnerAdapter, "multi-runner adapter capability");

  const branded = profile(
    {
      ...canonical,
      role: "branded",
      provider: "example",
      canonical_skill: "example",
      requires_adapter: true,
      completion: "provider_readback",
    },
    { type: "external-adapter", adapter: "example" },
  );
  assertNoFindings(branded, "branded provider operation");

  const nestedRoot = "/virtual/nested/X.yaml";
  const nestedProfile = profile(canonical, { type: "http", url: "https://example.test" });
  const graph = profile({ ...canonical, requires_adapter: true }, {
    type: "graph",
    graph: { steps: [{ id: "call", skill: "../nested" }] },
  });
  assertNoFindings(graph, "nested capability", {
    profiles: [[nestedRoot, nestedProfile]],
  });

  const unresolved = profile(canonical, {
    type: "graph",
    graph: { steps: [{ id: "missing", skill: "../missing" }] },
  });
  assertFindings(unresolved, "unresolved reference", ["does not resolve"]);

  const registrySkill = profile({ ...canonical, requires_adapter: true }, {
    type: "graph",
    graph: { steps: [{ id: "external", skill: "registry:example/operation@1.0.0" }] },
  });
  assertNoFindings(registrySkill, "registry capability");

  const cycle = profile(canonical, {
    type: "graph",
    graph: { steps: [{ id: "again", skill: ".", runner: "default" }] },
  });
  assertFindings(cycle, "cycle", ["cyclic runner reference"]);

  console.log("archetype-aware skill capability self-tests passed");
}

function profile(catalog, runner) {
  return {
    skill: "example",
    catalog,
    runners: { default: { default: true, ...runner } },
  };
}

function assertNoFindings(value, label, options = {}) {
  const findings = semanticCapabilityFindings(value, label, options);
  if (findings.length > 0) {
    throw new Error(`${label} should pass:\n${findings.join("\n")}`);
  }
}

function assertFindings(value, label, expected, options = {}) {
  const findings = semanticCapabilityFindings(value, label, options);
  for (const fragment of expected) {
    if (!findings.some((finding) => finding.includes(fragment))) {
      throw new Error(`${label} missing finding '${fragment}':\n${findings.join("\n")}`);
    }
  }
}
