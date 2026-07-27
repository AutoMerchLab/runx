const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u;
const ISSUE_REF = /^issues\/[1-9][0-9]*$/u;
const PULL_REF = /^pulls\/[1-9][0-9]*$/u;
const THREAD_REF = /^(issues|pulls)\/[1-9][0-9]*\/comments$/u;
const COMMENT_REF = /^issues\/comments\/[1-9][0-9]*$/u;
const MAX_BODY_LENGTH = 65_536;
const MAX_TITLE_LENGTH = 256;

export default function planSync(inputs) {
  const repo = text(inputs.repo);
  const direction = text(inputs.direction);
  const scope = text(inputs.scope);
  const resources = object(inputs.resources);
  const kind = text(resources.kind);
  const blockers = [];

  if (!REPOSITORY.test(repo)) blockers.push("repo must be owner/name");
  if (!["pull", "push"].includes(direction)) blockers.push("direction must be pull or push");
  if (!["read", "write"].includes(scope)) blockers.push("scope must be read or write");
  if (!["issues", "prs", "threads"].includes(kind)) blockers.push("resources.kind is invalid");

  const filters = normalizeFilters(kind, resources.filters, blockers);
  const refs = normalizeRefs(kind, resources.refs, blockers);
  const mutation = direction === "push"
    ? normalizeSingleMutation(kind, resources.mutations, blockers)
    : null;

  let decision = blockers.length === 0 ? "ready" : "blocked";
  if (direction === "push" && scope !== "write") {
    blockers.push("push requires requested write scope");
    decision = "refused";
  } else if (direction === "pull" && scope !== "read") {
    blockers.push("pull requires requested read scope");
    decision = "refused";
  } else if (blockers.length === 0 && direction === "push") {
    decision = "ready_for_approval";
  }

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
      mutation,
      diff_summary: mutation
        ? [{ ref: mutation.ref, op: mutation.op, fields: Object.keys(mutation.payload).sort() }]
        : [],
      provider_operation: providerOperation,
      scope_used: direction === "push" ? "repo.write" : "repo.read",
      gates: { approval_required: direction === "push", approval_ref: "" },
      provider_status: "not_called",
      blockers,
    },
  };
}

function normalizeFilters(kind, value, blockers) {
  const filters = object(value);
  const allowed = {
    issues: ["state", "labels", "since", "limit", "cursor"],
    prs: ["state", "base", "head", "sort", "direction", "limit", "cursor"],
    threads: ["thread_ref", "limit"],
  }[kind] ?? [];
  rejectUnknown(filters, allowed, "resources.filters", blockers);

  const fallback = 30;
  const limit = Number(filters.limit ?? fallback);
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
    blockers.push("resource limit must be from 1 to 100");
  }
  return { ...filters, limit };
}

function normalizeRefs(kind, value, blockers) {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > 100) {
    blockers.push("resources.refs must contain at most 100 resource refs");
    return [];
  }
  const refs = value.map((entry) => text(entry));
  const pattern = kind === "issues" ? ISSUE_REF : kind === "prs" ? PULL_REF : THREAD_REF;
  if (refs.some((ref) => !pattern.test(ref))) blockers.push(`resources.refs contains an invalid ${kind || "resource"} ref`);
  if (new Set(refs).size !== refs.length) blockers.push("resources.refs must not contain duplicates");
  return refs;
}

function normalizeSingleMutation(kind, value, blockers) {
  if (!Array.isArray(value) || value.length !== 1) {
    blockers.push("push requires exactly one typed mutation");
    return null;
  }
  const mutation = object(value[0]);
  rejectUnknown(mutation, ["ref", "op", "payload"], "mutation", blockers);
  const ref = text(mutation.ref);
  const op = text(mutation.op);
  const payload = object(mutation.payload);

  if (kind === "issues") validateIssueMutation(ref, op, payload, blockers);
  else if (kind === "prs") validatePullMutation(ref, op, payload, blockers);
  else if (kind === "threads") validateThreadMutation(ref, op, payload, blockers);

  return { ref, op, payload };
}

function validateIssueMutation(ref, op, payload, blockers) {
  if (!ISSUE_REF.test(ref)) blockers.push("issue mutation ref must be issues/<number>");
  if (op !== "update") blockers.push("issue mutation op must be update");
  rejectUnknown(
    payload,
    ["title", "body", "state", "state_reason", "labels", "assignees", "milestone"],
    "issue mutation payload",
    blockers,
  );
  requirePayload(payload, "issue mutation payload", blockers);
  optionalText(payload.title, MAX_TITLE_LENGTH, "payload.title", blockers, true);
  optionalText(payload.body, MAX_BODY_LENGTH, "payload.body", blockers);
  optionalEnum(payload.state, ["open", "closed"], "payload.state", blockers);
  optionalEnum(payload.state_reason, ["completed", "not_planned", "reopened"], "payload.state_reason", blockers);
  optionalStrings(payload.labels, 100, "payload.labels", blockers);
  optionalStrings(payload.assignees, 20, "payload.assignees", blockers);
  if (
    payload.milestone !== undefined
    && payload.milestone !== null
    && (!Number.isSafeInteger(payload.milestone) || payload.milestone < 1)
  ) {
    blockers.push("payload.milestone must be a positive integer or null");
  }
}

function validatePullMutation(ref, op, payload, blockers) {
  if (!PULL_REF.test(ref)) blockers.push("pull request mutation ref must be pulls/<number>");
  if (op !== "update") blockers.push("pull request mutation op must be update");
  rejectUnknown(
    payload,
    ["title", "body", "state", "base", "maintainer_can_modify"],
    "pull request mutation payload",
    blockers,
  );
  requirePayload(payload, "pull request mutation payload", blockers);
  optionalText(payload.title, MAX_TITLE_LENGTH, "payload.title", blockers, true);
  optionalText(payload.body, MAX_BODY_LENGTH, "payload.body", blockers);
  optionalEnum(payload.state, ["open", "closed"], "payload.state", blockers);
  if (
    payload.base !== undefined
    && (
      typeof payload.base !== "string"
      || !/^[A-Za-z0-9._/-]{1,255}$/u.test(payload.base)
    )
  ) {
    blockers.push("payload.base must be a valid branch name");
  }
  if (payload.maintainer_can_modify !== undefined && typeof payload.maintainer_can_modify !== "boolean") {
    blockers.push("payload.maintainer_can_modify must be boolean");
  }
}

function validateThreadMutation(ref, op, payload, blockers) {
  if (op === "comment" && !THREAD_REF.test(ref)) {
    blockers.push("new comment ref must be issues/<number>/comments or pulls/<number>/comments");
  } else if (op === "update" && !COMMENT_REF.test(ref)) {
    blockers.push("comment update ref must be issues/comments/<id>");
  } else if (!["comment", "update"].includes(op)) {
    blockers.push("thread mutation op must be comment or update");
  }
  rejectUnknown(payload, ["body"], "thread mutation payload", blockers);
  if (
    typeof payload.body !== "string"
    || !payload.body.trim()
    || payload.body.length > MAX_BODY_LENGTH
    || payload.body.includes("\u0000")
  ) {
    blockers.push(`payload.body must be non-empty text of at most ${MAX_BODY_LENGTH} characters`);
  }
}

function requirePayload(payload, label, blockers) {
  if (Object.keys(payload).length === 0) blockers.push(`${label} requires at least one field`);
}

function rejectUnknown(value, allowed, label, blockers) {
  const unknown = Object.keys(value).filter((field) => !allowed.includes(field));
  if (unknown.length > 0) blockers.push(`${label} contains unsupported fields: ${unknown.join(", ")}`);
}

function optionalText(value, maximum, label, blockers, nonBlank = false) {
  if (value === undefined) return;
  if (
    typeof value !== "string"
    || value.length > maximum
    || (nonBlank && !value.trim())
    || value.includes("\u0000")
  ) {
    blockers.push(`${label} must be ${nonBlank ? "non-empty " : ""}text of at most ${maximum} characters`);
  }
}

function optionalEnum(value, allowed, label, blockers) {
  if (value !== undefined && !allowed.includes(value)) blockers.push(`${label} must be one of ${allowed.join(", ")}`);
}

function optionalStrings(value, maximum, label, blockers) {
  if (value === undefined) return;
  if (
    !Array.isArray(value)
    || value.length > maximum
    || value.some((entry) =>
      typeof entry !== "string"
      || !entry.trim()
      || entry.length > 100
      || entry.includes("\u0000"))
    || new Set(value).size !== value.length
  ) {
    blockers.push(`${label} must contain at most ${maximum} unique non-empty strings`);
  }
}

function object(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}
