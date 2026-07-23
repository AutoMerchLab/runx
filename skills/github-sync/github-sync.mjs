export default function planSync(inputs) {
  const repo = text(inputs.repo);
  const direction = text(inputs.direction);
  const scope = text(inputs.scope);
  const resources = object(inputs.resources);
  const kind = text(resources.kind);
  const filters = object(resources.filters);
  const refs = strings(resources.refs);
  const mutations = Array.isArray(resources.mutations) ? resources.mutations : [];
  const blockers = [];
  let decision = "ready";

  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(repo)) blockers.push("repo must be owner/name");
  if (!["pull", "push"].includes(direction)) blockers.push("direction must be pull or push");
  if (!["read", "write"].includes(scope)) blockers.push("scope must be read or write");
  if (!["issues", "prs", "threads"].includes(kind)) blockers.push("resources.kind is invalid");
  const limit = Number(filters.limit ?? Math.max(refs.length, mutations.length, 30));
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) blockers.push("resource limit must be from 1 to 100");
  if (refs.length > 100 || mutations.length > 100) blockers.push("resource set exceeds 100 items");
  if (direction === "push" && scope !== "write") {
    blockers.push("push requires requested write scope");
    decision = "refused";
  }
  if (direction === "pull" && scope !== "read") blockers.push("pull requires read scope");
  if (direction === "push" && mutations.length === 0) blockers.push("push requires digest-only mutations");
  for (const mutation of mutations) {
    if (!text(mutation?.ref) || !text(mutation?.op) || !/^sha256:[0-9a-f]{64}$/u.test(text(mutation?.digest))) {
      blockers.push("each mutation requires ref, op, and sha256 digest");
    }
    if (Object.prototype.hasOwnProperty.call(object(mutation), "body")) {
      blockers.push("raw mutation bodies are forbidden");
    }
  }
  if (blockers.length > 0 && decision !== "refused") decision = "blocked";
  if (blockers.length === 0 && direction === "push") decision = "ready_for_approval";

  const providerOperation = ["pull", "push"].includes(direction)
    ? ({
        issues: direction === "push" ? "issues.write" : "issues.read",
        prs: direction === "push" ? "pullrequests.write" : "pullrequests.read",
        threads: direction === "push" ? "threads.write" : "threads.read",
      }[kind] || "")
    : "";

  return {
    sync_plan: {
      decision,
      repo,
      direction,
      resource_selector: { kind, filters, refs },
      resources_touched: [],
      diff_summary: mutations.map((mutation) => ({
        ref: text(mutation.ref),
        op: text(mutation.op),
        digest: text(mutation.digest),
      })),
      provider_operation: providerOperation,
      scope_used: direction === "push" ? "repo:write" : "repo:read",
      gates: { approval_required: direction === "push", approval_ref: "" },
      provider_status: "not_called",
      blockers,
    },
  };
}

function object(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function strings(value) {
  return Array.isArray(value)
    ? value.filter((entry) => typeof entry === "string").map((entry) => entry.trim()).filter(Boolean)
    : [];
}
