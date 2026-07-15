#!/usr/bin/env node
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

if (process.argv.includes("--self-test")) {
  runSelfTests();
  process.exit(0);
}

const staticFindings = checkCurrentCatalog();
const base = resolveBase();
const driftFindings = base ? checkVersionDrift(base) : [];
const findings = [...staticFindings, ...driftFindings];

if (findings.length > 0) {
  for (const finding of findings) console.error(finding);
  process.exit(1);
}

const comparison = base ? ` against ${base}` : " (version comparison skipped: no Git base)";
console.log(`skill catalog version and graph-input checks ok${comparison}`);

function checkCurrentCatalog() {
  const findings = [];
  const skillsRoot = path.join(root, "skills");
  for (const entry of readdirSync(skillsRoot, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const profilePath = path.join(skillsRoot, entry.name, "X.yaml");
    if (!existsSync(profilePath)) continue;
    const source = readFileSync(profilePath, "utf8");
    let profile;
    try {
      profile = parseYaml(source);
    } catch (error) {
      findings.push(`skills/${entry.name}/X.yaml: invalid YAML: ${error.message}`);
      continue;
    }
    if (!isSemver(profile?.version)) {
      findings.push(`skills/${entry.name}/X.yaml: version must be quoted semantic version x.y.z`);
    } else if (!source.split("\n").some((line) => line === `version: "${profile.version}"`)) {
      findings.push(`skills/${entry.name}/X.yaml: version must be quoted as "${profile.version}"`);
    }
    for (const match of source.matchAll(/\{\{\s*([A-Za-z0-9_.]+)\s*\}\}/gu)) {
      findings.push(
        `skills/${entry.name}/X.yaml: retired graph input binding ${match[0]}; use $input.${match[1]}`,
      );
    }
  }
  return findings;
}

function checkVersionDrift(base) {
  const changedBySkill = new Map();
  for (const changedPath of changedSkillPaths(base)) {
    const parts = changedPath.split("/");
    if (parts.length < 3 || parts[0] !== "skills") continue;
    const skill = parts[1];
    const relative = parts.slice(2).join("/");
    if (!skill || !relative) continue;
    const currentScripts = consumedScripts("current", base, skill);
    const baseScripts = consumedScripts("base", base, skill);
    if (!isPublishableRelative(relative, currentScripts) && !isPublishableRelative(relative, baseScripts)) {
      continue;
    }
    const paths = changedBySkill.get(skill) ?? [];
    paths.push(changedPath);
    changedBySkill.set(skill, paths);
  }

  const findings = [];
  for (const [skill, changedPaths] of changedBySkill) {
    const baseVersion = skillVersion("base", base, skill);
    const currentVersion = skillVersion("current", base, skill);
    const finding = versionFinding(skill, baseVersion, currentVersion, changedPaths);
    if (finding) findings.push(finding);
  }
  return findings;
}

function changedSkillPaths(base) {
  const output = git(["diff", "--name-status", "--find-renames", base, "--", "skills"]);
  const paths = new Set();
  for (const line of output.split("\n")) {
    if (!line.trim()) continue;
    const fields = line.split("\t");
    const status = fields[0] ?? "";
    if (status.startsWith("R") || status.startsWith("C")) {
      if (fields[1]) paths.add(fields[1]);
      if (fields[2]) paths.add(fields[2]);
    } else if (fields[1]) {
      paths.add(fields[1]);
    }
  }
  for (const untracked of git(["ls-files", "--others", "--exclude-standard", "--", "skills"]).split("\n")) {
    if (untracked) paths.add(untracked);
  }
  return [...paths].sort();
}

function isPublishableRelative(relative, consumedScripts) {
  if (consumedScripts.has(relative)) return true;
  const parts = relative.split("/");
  if (parts.some((part) => !part || part.startsWith("."))) return false;
  if (parts.some((part) => ["assets", "dist", "fixtures", "node_modules", "src", "target"].includes(part))) {
    return false;
  }
  const fileName = parts.at(-1) ?? "";
  if (parts.includes("references")) return fileName.endsWith(".md");
  return ["SKILL.md", "X.yaml", "manifest.json", "run.mjs", "run.js", "harness.mjs", "harness.js"]
    .includes(fileName);
}

function consumedScripts(side, base, skill) {
  const source = readSide(side, base, `skills/${skill}/X.yaml`);
  if (!source) return new Set();
  let profile;
  try {
    profile = parseYaml(source);
  } catch {
    return new Set();
  }
  const scripts = new Set();
  visitValues(profile, (value) => {
    const normalized = normalizeScriptPath(value);
    if (normalized) scripts.add(normalized);
  });
  return scripts;
}

function visitValues(value, visit) {
  if (typeof value === "string") {
    visit(value);
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) visitValues(item, visit);
    return;
  }
  if (value && typeof value === "object") {
    for (const item of Object.values(value)) visitValues(item, visit);
  }
}

function normalizeScriptPath(value) {
  const normalized = value.trim().replace(/^\.\//u, "");
  if (!(normalized.endsWith(".mjs") || normalized.endsWith(".js"))) return null;
  if (normalized.startsWith("/") || normalized.includes("\\")) return null;
  const parts = normalized.split("/");
  if (parts.some((part) => !part || part === "." || part === "..")) return null;
  return normalized;
}

function skillVersion(side, base, skill) {
  const source = readSide(side, base, `skills/${skill}/X.yaml`);
  if (!source) return null;
  try {
    const version = parseYaml(source)?.version;
    return isSemver(version) ? version : null;
  } catch {
    return null;
  }
}

function versionFinding(skill, baseVersion, currentVersion, changedPaths) {
  if (baseVersion === null || currentVersion === null) return null;
  if (compareSemver(currentVersion, baseVersion) > 0) return null;
  return [
    `skills/${skill}: publishable package material changed without a version increase`,
    `  version: ${baseVersion} -> ${currentVersion}`,
    ...changedPaths.map((changedPath) => `  changed: ${changedPath}`),
  ].join("\n");
}

function isSemver(value) {
  return typeof value === "string" && /^\d+\.\d+\.\d+$/u.test(value);
}

function compareSemver(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1) {
    const difference = leftParts[index] - rightParts[index];
    if (difference !== 0) return difference;
  }
  return 0;
}

function readSide(side, base, relativePath) {
  if (side === "current") {
    const absolutePath = path.join(root, relativePath);
    return existsSync(absolutePath) ? readFileSync(absolutePath, "utf8") : null;
  }
  try {
    return git(["show", `${base}:${relativePath}`]);
  } catch {
    return null;
  }
}

function resolveBase() {
  const baseArgIndex = process.argv.indexOf("--base");
  const requested = baseArgIndex >= 0 ? process.argv[baseArgIndex + 1] : undefined;
  const candidates = [requested, process.env.RUNX_SKILL_VERSION_BASE, process.env.GITHUB_BASE_SHA, "HEAD"];
  for (const candidate of candidates) {
    if (!candidate) continue;
    try {
      const commit = git(["rev-parse", "--verify", `${candidate}^{commit}`]).trim();
      return git(["merge-base", "HEAD", commit]).trim();
    } catch {
      // Try the next source. Static catalog checks still run without a base.
    }
  }
  return null;
}

function git(args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

function runSelfTests() {
  const empty = new Set();
  assert(isPublishableRelative("SKILL.md", empty), "SKILL.md is publishable");
  assert(isPublishableRelative("references/operator.md", empty), "reference markdown is publishable");
  assert(!isPublishableRelative("fixtures/evidence.json", empty), "ordinary fixtures are not package material");
  assert(
    isPublishableRelative("fixtures/helper.mjs", new Set(["fixtures/helper.mjs"])),
    "an explicitly consumed harness helper is package material",
  );
  assert(versionFinding("new-skill", null, "0.1.0", ["skills/new-skill/SKILL.md"]) === null, "new skill is allowed");
  assert(versionFinding("old-skill", "0.1.0", null, ["skills/old-skill/SKILL.md"]) === null, "deleted skill is allowed");
  assert(versionFinding("same", "0.1.0", "0.1.0", ["skills/same/SKILL.md"]) !== null, "unchanged version fails");
  assert(versionFinding("bumped", "0.1.0", "0.1.1", ["skills/bumped/SKILL.md"]) === null, "increased version passes");
  console.log("skill version drift self-tests ok");
}

function assert(condition, message) {
  if (!condition) throw new Error(`self-test failed: ${message}`);
}
