export function normalizeIssueEvidence(inputs) {
  const operation = object(inputs.provider_operation);
  const issue = object(operation.result);
  return {
    issue_evidence: {
      schema: "runx.issue_to_pr.issue_evidence.v1",
      repository: text(issue.repository),
      number: text(issue.number),
      title: text(issue.title),
      state: text(issue.state),
      body: text(issue.body),
      url: text(issue.url),
      labels: strings(issue.labels),
      assignees: strings(issue.assignees),
      source: {
        provider: text(operation.provider),
        transport: text(operation.transport),
        principal_ref: text(operation.principal_ref),
        readback_ref: text(operation.readback_ref),
      },
      checkout_resolved: text(inputs.requested_repository) === ".",
    },
  };
}

export function admitHostResult(inputs) {
  const host = object(inputs.host_result);
  const issue = object(inputs.issue_evidence);
  const tests = Array.isArray(host.tests) ? host.tests : [];
  const errors = [];
  if (text(host.repository) !== text(issue.repository)) {
    errors.push("host result repository does not match issue evidence");
  }
  if (text(host.issue_number) !== text(issue.number)) {
    errors.push("host result issue number does not match issue evidence");
  }
  if (text(host.status) !== "completed") errors.push("host work did not complete");
  if (tests.length === 0 || tests.some((test) => text(test?.status) !== "passed")) {
    errors.push("host work has no complete passing test evidence");
  }
  const finalization = object(host.finalization);
  if (text(finalization.status) !== "passed" || Number(finalization.invocation_count) !== 1) {
    errors.push("scafld finalize must pass exactly once");
  }
  if (!text(finalization.receipt_ref)) errors.push("scafld finalize receipt is missing");
  const publication = object(host.publication);
  const requested = text(publication.decision) === "ready";
  if (requested) {
    for (const field of ["title", "body", "head", "base", "idempotency_key"]) {
      if (!text(publication[field])) errors.push(`publication ${field} is missing`);
    }
  }
  return {
    host_admission: {
      path: errors.length > 0 ? "blocked" : requested ? "publish" : "hold",
      errors,
    },
  };
}

export function finalizeIssueToPr(inputs) {
  const host = object(inputs.host_result);
  const admission = object(inputs.host_admission);
  const publication = object(inputs.publication_result);
  const readback = object(inputs.readback_result);
  const published = text(publication.status) === "success" && text(readback.status) === "success";
  const errors = Array.isArray(admission.errors) ? [...admission.errors] : [];
  if (publication.status !== undefined && !published) errors.push("pull-request publication did not verify");
  const complete = errors.length === 0;
  return {
    issue_to_pr_result: {
      schema: "runx.issue_to_pr.result.v1",
      status: complete ? "completed" : "blocked",
      outcome: complete ? "completed" : "blocked",
      repository: text(host.repository),
      issue_number: text(host.issue_number),
      branch: text(host.branch),
      tested: Array.isArray(host.tests) && host.tests.length > 0
        && host.tests.every((test) => text(test?.status) === "passed"),
      finalization_ref: text(host.finalization?.receipt_ref),
      publication: {
        status: published ? "published" : "not_requested",
        pr_number: text(readback.result?.number),
        url: text(readback.result?.url),
        readback_ref: text(readback.readback_ref),
      },
      errors,
    },
  };
}

function object(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function text(value) {
  return typeof value === "string" ? value : "";
}

function strings(value) {
  return Array.isArray(value) ? value.filter((entry) => typeof entry === "string") : [];
}
