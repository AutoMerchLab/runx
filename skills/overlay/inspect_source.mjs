import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const MAX_BYTES = 512 * 1024;
const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const root = path.resolve(process.env.RUNX_CWD || process.cwd());
const scriptRoot = path.dirname(fileURLToPath(import.meta.url));
const decision = { value: "ready", findings: [] };
const sourceLocation = resolveSource(inputs.skill_path);
const relativePath = sourceLocation.label;
const absolutePath = sourceLocation.absolute;
const upstream = requiredRecord(inputs.upstream, "upstream");
const registry = requiredRecord(inputs.registry, "registry");

let contents = Buffer.alloc(0);
let frontmatter = {};
try {
  if (!absolutePath) throw new Error("skill_path must name a workspace or skill-local SKILL.md");
  const stat = fs.lstatSync(absolutePath);
  if (stat.isSymbolicLink() || !stat.isFile() || path.basename(absolutePath) !== "SKILL.md") throw new Error("skill_path must name a regular SKILL.md file");
  if (stat.size > MAX_BYTES) throw new Error("SKILL.md exceeds the 512 KiB inspection limit");
  contents = fs.readFileSync(absolutePath);
  frontmatter = parseFrontmatter(contents.toString("utf8"));
} catch (error) {
  reject("source.unavailable", "skill_path", boundedMessage(error));
}

const observedBlobSha = gitBlobSha(contents);
const observedSha256 = sha256(contents);
validateUpstream(upstream, observedBlobSha);
validateRegistry(registry);

const skillName = stringValue(frontmatter.name) || "unknown";
const owner = safeSegment(registry.owner, "registry.owner");
if (!/^[a-z0-9][a-z0-9-]*$/u.test(skillName)) reject("source.invalid_name", "frontmatter.name", "The upstream skill name must be a lowercase package segment.");

process.stdout.write(`${JSON.stringify({
  source_evidence: {
    decision: decision.value,
    findings: decision.findings,
    source: {
      path: relativePath,
      name: skillName,
      description: stringValue(frontmatter.description) || "Upstream skill bound by Runx.",
      bytes: contents.length,
      sha256: `sha256:${observedSha256}`,
      git_blob_sha: observedBlobSha,
    },
    upstream,
    registry,
    binding_path: `bindings/${owner}/${skillName}`,
    tags: uniqueStrings(inputs.tags),
    publication: isRecord(inputs.publication) ? inputs.publication : { status: "not_published" },
  },
}, null, 2)}\n`);

function validateUpstream(value, observedBlobSha) {
  if (value.host !== "github.com") reject("upstream.unsupported_host", "upstream.host", "Native upstream bindings currently require github.com provenance.");
  const owner = safeSegment(value.owner, "upstream.owner");
  const repo = safeSegment(value.repo, "upstream.repo");
  if (value.path !== "SKILL.md") reject("upstream.invalid_path", "upstream.path", "The upstream source-of-truth path must be SKILL.md.");
  const commit = hex(value.commit, 40, "upstream.commit");
  const blobSha = hex(value.blob_sha, 40, "upstream.blob_sha");
  if (blobSha && observedBlobSha && blobSha !== observedBlobSha) reject("upstream.blob_mismatch", "upstream.blob_sha", "The local SKILL.md does not match the pinned upstream Git blob.");
  if (value.source_of_truth !== true) reject("upstream.not_source_of_truth", "upstream.source_of_truth", "The binding requires an upstream source-of-truth assertion.");
  pinnedUrl(value.html_url, [owner, repo, commit, "SKILL.md"], "upstream.html_url");
  pinnedUrl(value.raw_url, [owner, repo, commit, "SKILL.md"], "upstream.raw_url");
}

function validateRegistry(value) {
  safeSegment(value.owner, "registry.owner");
  if (!new Set(["community", "verified", "first_party"]).has(value.trust_tier)) reject("registry.invalid_trust_tier", "registry.trust_tier", "Registry trust tier must be community, verified, or first_party.");
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/u.test(stringValue(value.version) || "")) reject("registry.invalid_version", "registry.version", "Registry version must be an immutable package segment.");
}

function reject(code, pathValue, message) {
  decision.value = "reject";
  decision.findings.push({ code, path: pathValue, message });
}

function pinnedUrl(value, parts, field) {
  const parsed = stringValue(value);
  if (!parsed || parts.some((part) => part && !parsed.includes(part))) reject("upstream.unpinned_url", field, "Pinned upstream URLs must include owner, repo, commit, and SKILL.md path.");
}

function parseFrontmatter(value) {
  const match = value.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/u);
  if (!match) throw new Error("SKILL.md must start with YAML frontmatter");
  const result = {};
  for (const line of match[1].split(/\r?\n/u)) {
    const field = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/u);
    if (field) result[field[1]] = field[2].replace(/^['"]|['"]$/gu, "").trim();
  }
  if (!result.name) throw new Error("SKILL.md frontmatter is missing name");
  return result;
}

function gitBlobSha(value) {
  return createHash("sha1").update(Buffer.from(`blob ${value.length}\0`)).update(value).digest("hex");
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function normalizeRelative(value) {
  const normalized = path.posix.normalize(value.replaceAll("\\", "/"));
  if (!normalized || normalized === "." || normalized === ".." || normalized.startsWith("../") || path.posix.isAbsolute(normalized)) throw new Error("skill_path must stay inside the workspace");
  return normalized;
}

function resolveSource(value) {
  const raw = stringValue(value);
  if (!raw) {
    reject("source.missing", "skill_path", "A pinned local SKILL.md path is required.");
    return { label: "", absolute: null };
  }
  if (raw.startsWith("skill://")) {
    const relative = normalizeRelative(raw.slice("skill://".length));
    return boundedLocation(scriptRoot, relative, `skill://${relative}`);
  }
  const relative = normalizeRelative(raw);
  return boundedLocation(root, relative, relative);
}

function boundedLocation(base, relative, label) {
  const absolute = path.resolve(base, relative);
  const relation = path.relative(base, absolute);
  if (!relation || relation === ".." || relation.startsWith(`..${path.sep}`) || path.isAbsolute(relation)) throw new Error("skill_path escapes its source root");
  return { label, absolute };
}

function safeSegment(value, field) {
  const parsed = stringValue(value);
  if (!parsed || !/^[a-z0-9][a-z0-9-]*$/u.test(parsed)) {
    reject("binding.invalid_segment", field, `${field} must be a lowercase package segment.`);
    return "invalid";
  }
  return parsed;
}

function hex(value, length, field) {
  const parsed = stringValue(value);
  if (!parsed || !new RegExp(`^[a-f0-9]{${length}}$`, "iu").test(parsed)) {
    reject("binding.invalid_digest", field, `${field} must be a ${length}-character hex digest.`);
    return null;
  }
  return parsed.toLowerCase();
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
  return (error instanceof Error ? error.message : "Source inspection failed").replace(/\s+/gu, " ").slice(0, 160);
}
