const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const index = requiredRecord(inputs.evidence_index, "evidence_index");
const draft = requiredRecord(inputs.prior_art_draft, "prior_art_draft");
const requestedDecision = enumValue(draft.decision, ["ready", "needs_more_evidence"], "decision");
const catalog = records(index.catalog);
const inspected = records(index.inspected_sources);
const missing = records(index.missing_sources);
const allowedPaths = new Set([...catalog, ...inspected].map((entry) => stringValue(entry.path)).filter(Boolean));
const catalogNames = new Set(catalog.map((entry) => stringValue(entry.name)).filter(Boolean));
const validationFindings = missing.map((entry) => ({
  code: "source.missing",
  path: stringValue(entry.path) || "unknown",
  message: "A requested source was not available to the deterministic inspection step.",
}));

const findings = records(draft.findings).map((entry, position) => {
  const source = requiredString(entry.source, `findings[${position}].source`);
  const confidence = enumValue(entry.confidence, ["verified", "likely", "unverified"], `findings[${position}].confidence`);
  if (confidence === "verified" && !allowedPaths.has(source)) {
    validationFindings.push({ code: "finding.unbound_source", path: source, message: "A verified finding must cite an inspected source." });
  }
  return {
    claim: requiredString(entry.claim, `findings[${position}].claim`),
    source,
    relevance: requiredString(entry.relevance, `findings[${position}].relevance`),
    confidence,
  };
});

const catalogFit = requiredRecord(draft.catalog_fit, "catalog_fit");
const adjacentSkills = strings(catalogFit.adjacent_skills);
for (const skill of adjacentSkills) {
  if (!catalogNames.has(skill)) validationFindings.push({ code: "catalog.unknown_skill", path: skill, message: "Adjacent skill is absent from the inspected catalog." });
}
const sources = records(draft.sources).map((entry, position) => {
  const sourcePath = requiredString(entry.path, `sources[${position}].path`);
  if (!allowedPaths.has(sourcePath)) validationFindings.push({ code: "source.unbound", path: sourcePath, message: "Source is absent from the inspection index." });
  return { path: sourcePath, kind: stringValue(entry.kind) || "repo-document" };
});

const decision = requestedDecision === "ready" && validationFindings.length === 0 ? "ready" : "needs_more_evidence";
process.stdout.write(`${JSON.stringify({
  decision,
  findings,
  catalog_fit: {
    decision: enumValue(catalogFit.decision, ["reuse", "amend", "new_work", "stop"], "catalog_fit.decision"),
    adjacent_skills: adjacentSkills,
    rationale: requiredString(catalogFit.rationale, "catalog_fit.rationale"),
  },
  quality_bar: record(draft.quality_bar),
  recommended_flow: records(draft.recommended_flow),
  sources,
  risks: records(draft.risks),
  evidence: {
    cited_paths: [...new Set([...findings.map((entry) => entry.source), ...sources.map((entry) => entry.path)])].sort(),
    inspected_sources: inspected,
    catalog_refs: adjacentSkills.map((name) => catalog.find((entry) => entry.name === name)).filter(Boolean),
  },
  validation: {
    status: decision === "ready" ? "pass" : "hold",
    findings: validationFindings,
  },
}, null, 2)}\n`);

function records(value) {
  return Array.isArray(value) ? value.map(record) : [];
}

function strings(value) {
  return Array.isArray(value) ? [...new Set(value.map(stringValue).filter(Boolean))] : [];
}

function record(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function requiredRecord(value, field) {
  const parsed = record(value);
  if (Object.keys(parsed).length === 0) throw new Error(`${field} must be a non-empty object`);
  return parsed;
}

function enumValue(value, allowed, field) {
  if (!allowed.includes(value)) throw new Error(`${field} must be one of ${allowed.join(", ")}`);
  return value;
}

function requiredString(value, field) {
  const parsed = stringValue(value);
  if (!parsed) throw new Error(`${field} must be a non-empty string`);
  return parsed;
}

function stringValue(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}
