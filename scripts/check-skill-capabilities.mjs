#!/usr/bin/env node
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import YAML from "yaml";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const selfTest = process.argv.includes("--self-test");

if (selfTest) {
  const valid = capabilityFindings({
    catalog: {
      visibility: "public",
      role: "canonical",
      execution: "plan",
      completion: "plan",
      requires_adapter: true,
      approval: "required",
    },
    runners: { plan: { type: "agent-task" } },
  }, "valid");
  const invalid = capabilityFindings({
    catalog: { visibility: "public", role: "branded" },
    runners: { plan: { type: "agent-task" } },
  }, "invalid");
  if (valid.length > 0 || invalid.length !== 4) {
    throw new Error("skill capability gate self-test failed");
  }
  console.log("skill capability gate self-tests ok");
  process.exit(0);
}

const findings = [];
for (const name of readdirSync(path.join(root, "skills"), { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort()) {
  const profilePath = path.join(root, "skills", name, "X.yaml");
  let profile;
  try {
    profile = YAML.parse(readFileSync(profilePath, "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT") continue;
    throw error;
  }
  findings.push(...capabilityFindings(profile, path.relative(root, profilePath)));
}
if (findings.length > 0) {
  for (const finding of findings) console.error(finding);
  process.exit(1);
}
console.log("public agent-only skill capability metadata ok");

function capabilityFindings(profile, label) {
  const catalog = profile?.catalog ?? {};
  const runners = Object.values(profile?.runners ?? {});
  const publicActionSurface = catalog.visibility === "public"
    && ["canonical", "branded"].includes(catalog.role);
  const agentOnly = runners.length > 0
    && runners.every((runner) => ["agent", "agent-task"].includes(runner?.type ?? runner?.source?.type));
  if (!publicActionSurface || !agentOnly) return [];
  return ["execution", "completion", "requires_adapter", "approval"]
    .filter((field) => catalog[field] === undefined)
    .map((field) => `${label}: public canonical/branded agent-only skill must declare catalog.${field}`);
}
