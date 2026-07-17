import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const MAX_FILES = 50;
const MAX_FILE_BYTES = 256 * 1024;
const MAX_TOTAL_BYTES = 1024 * 1024;
const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const root = path.resolve(process.env.RUNX_CWD || process.cwd());
const skillsRoot = path.join(root, "skills");
const catalog = inspectCatalog(skillsRoot, root);
const requested = requestedPaths(inputs).slice(0, MAX_FILES);
const inspectedSources = [];
const missingSources = [];
let totalBytes = 0;

for (const relative of requested) {
  try {
    const absolute = resolveBounded(root, relative);
    const stat = fs.lstatSync(absolute);
    if (stat.isSymbolicLink() || !stat.isFile()) throw new Error("source must be a regular file");
    if (stat.size > MAX_FILE_BYTES || totalBytes + stat.size > MAX_TOTAL_BYTES) throw new Error("source exceeds inspection byte limit");
    const contents = fs.readFileSync(absolute);
    totalBytes += contents.length;
    inspectedSources.push({
      path: relative,
      kind: relative.endsWith("/X.yaml") ? "skill-manifest" : "repo-document",
      bytes: contents.length,
      digest: `sha256:${createHash("sha256").update(contents).digest("hex")}`,
    });
  } catch (error) {
    missingSources.push({
      path: relative,
      reason: error instanceof Error ? boundedReason(error.message) : "source could not be inspected",
    });
  }
}

process.stdout.write(`${JSON.stringify({
  evidence_index: {
    schema: "runx.prior_art.evidence_index.v1",
    workspace: "local",
    catalog,
    inspected_sources: inspectedSources,
    missing_sources: missingSources,
    limits: { max_files: MAX_FILES, max_file_bytes: MAX_FILE_BYTES, max_total_bytes: MAX_TOTAL_BYTES },
  },
}, null, 2)}\n`);

function inspectCatalog(directory, workspaceRoot) {
  if (!fs.existsSync(directory)) return [];
  return fs.readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && !entry.name.startsWith("."))
    .sort((left, right) => left.name.localeCompare(right.name))
    .flatMap((entry) => {
      const manifestPath = path.join(directory, entry.name, "X.yaml");
      if (!fs.existsSync(manifestPath)) return [];
      try {
        const document = fs.readFileSync(manifestPath);
        return [{
          name: entry.name,
          path: path.relative(workspaceRoot, manifestPath).split(path.sep).join("/"),
          digest: `sha256:${createHash("sha256").update(document).digest("hex")}`,
          kind: "skill-manifest",
        }];
      } catch {
        return [];
      }
    });
}

function requestedPaths(value) {
  const result = new Set(strings(value.source_paths).map(normalizeRelative));
  const decomposition = record(value.decomposition);
  for (const skill of records(decomposition.required_skills)) {
    if (skill.exists === true) addSkillManifest(result, skill.name);
  }
  for (const step of records(decomposition.orchestration_steps)) addSkillManifest(result, step.skill);
  return [...result].sort();
}

function addSkillManifest(target, value) {
  const name = stringValue(value)?.replace(/^\.\.\//u, "");
  if (name && /^[a-z0-9][a-z0-9-]*$/u.test(name)) target.add(`skills/${name}/X.yaml`);
}

function normalizeRelative(value) {
  const normalized = path.posix.normalize(value.replaceAll("\\", "/"));
  if (!normalized || normalized === "." || normalized === ".." || normalized.startsWith("../") || path.posix.isAbsolute(normalized)) {
    throw new Error(`source path must stay inside the workspace: ${value}`);
  }
  return normalized;
}

function resolveBounded(workspaceRoot, relative) {
  const resolved = path.resolve(workspaceRoot, relative);
  const relation = path.relative(workspaceRoot, resolved);
  if (!relation || relation === ".." || relation.startsWith(`..${path.sep}`) || path.isAbsolute(relation)) {
    throw new Error("source path escapes the workspace");
  }
  return resolved;
}

function boundedReason(value) {
  return String(value).replace(/\s+/gu, " ").trim().slice(0, 120);
}

function records(value) {
  return Array.isArray(value) ? value.map(record) : [];
}

function record(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function strings(value) {
  return Array.isArray(value) ? value.map(stringValue).filter(Boolean) : [];
}

function stringValue(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}
