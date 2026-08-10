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
  if (!text(host.repo_root)) errors.push("host repository root is missing");
  if (!exactCommit(host.commit)) errors.push("host result does not name an exact commit");
  if (!text(host.branch)) errors.push("host branch is missing");
  if (!Array.isArray(host.files) || host.files.length === 0) {
    errors.push("host work has no changed-file evidence");
  }
  if (tests.length === 0 || tests.some((test) => text(test?.status) !== "passed")) {
    errors.push("host work has no complete passing test evidence");
  }
  const finalization = object(host.finalization);
  if (!text(finalization.receipt_path)) errors.push("scafld finalize receipt path is missing");
  if (!digest(finalization.contract_digest)) errors.push("scafld contract digest is invalid");
  const publication = object(host.publication);
  const requested = text(publication.decision) === "ready";
  if (requested) {
    for (const field of ["title", "body", "head", "base", "idempotency_key"]) {
      if (!text(publication[field])) errors.push(`publication ${field} is missing`);
    }
    if (text(publication.head) !== text(host.branch)) {
      errors.push("publication head does not match the tested host branch");
    }
  }
  return {
    host_admission: {
      path: errors.length > 0 ? "blocked" : requested ? "publish" : "finalized_local",
      errors,
    },
  };
}

export function finalizeIssueToPr(inputs) {
  const host = object(inputs.host_result);
  const admission = object(inputs.host_admission);
  const verification = object(inputs.receipt_verification);
  const publication = object(inputs.publication_result);
  const errors = Array.isArray(admission.errors) ? [...admission.errors] : [];
  const verified = verification.verified === true
    && text(verification.verdict) === "pass"
    && text(verification.target) === text(host.commit)
    && text(verification.contract_digest) === text(host.finalization?.contract_digest)
    && Boolean(text(verification.receipt_ref));
  if (errors.length === 0 && !verified) errors.push("scafld finalization did not verify");
  const publicationRequested = text(host.publication?.decision) === "ready";
  const publicationResult = object(publication.result);
  const published = publicationRequested
    && text(publication.status) === "success"
    && text(publication.operation) === "pullrequest.publish"
    && text(publicationResult.repository) === text(host.repository)
    && text(publicationResult.published_commit) === text(host.commit)
    && text(publicationResult.head) === text(host.publication?.head)
    && text(publicationResult.base) === text(host.publication?.base)
    && Boolean(text(publication.readback_ref));
  if (publicationRequested && !published) errors.push("exact-ref pull-request publication did not verify");
  const complete = errors.length === 0;
  const outcome = complete ? (published ? "published" : "finalized_local") : "blocked";
  return {
    issue_to_pr_result: {
      schema: "runx.issue_to_pr.result.v1",
      status: complete ? "completed" : "blocked",
      outcome,
      repository: text(host.repository),
      issue_number: text(host.issue_number),
      branch: text(host.branch),
      commit: text(host.commit),
      tested: Array.isArray(host.tests) && host.tests.length > 0
        && host.tests.every((test) => text(test?.status) === "passed"),
      finalization: {
        status: verified ? "verified" : "unverified",
        receipt_ref: text(verification.receipt_ref),
        contract_digest: text(host.finalization?.contract_digest),
        target: text(host.commit),
      },
      publication: {
        status: published ? "published" : publicationRequested ? "failed" : "not_requested",
        pr_number: scalarText(publicationResult.number),
        url: text(publicationResult.url),
        readback_ref: text(publication.readback_ref),
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

function scalarText(value) {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return `${value}`;
  return "";
}

function exactCommit(value) {
  return typeof value === "string"
    && (value.length === 40 || value.length === 64)
    && /^[0-9a-f]+$/.test(value);
}

function digest(value) {
  return typeof value === "string" && /^sha256:[0-9a-f]{64}$/.test(value);
}

function strings(value) {
  return Array.isArray(value) ? value.filter((entry) => typeof entry === "string") : [];
}
