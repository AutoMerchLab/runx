const ALLOWED_DIMENSIONS = new Set([
  "query",
  "page",
  "country",
  "device",
  "date",
  "hour",
  "searchAppearance",
]);

const ALLOWED_SEARCH_TYPES = new Set([
  "web",
  "image",
  "video",
  "news",
  "googleNews",
  "discover",
]);

const ALLOWED_DATA_STATES = new Set(["final", "all", "hourly_all"]);

export function normalizePerformance(inputs) {
  const supplied = object(inputs.provider_result);
  const expected = object(inputs.request);
  const findings = [];

  const property = text(supplied.property || expected.property);
  const startDate = text(supplied.start_date || expected.start_date);
  const endDate = text(supplied.end_date || expected.end_date);
  const dimensions = stringArray(
    Array.isArray(supplied.dimensions) ? supplied.dimensions : expected.dimensions,
  );
  const searchType = text(supplied.search_type || expected.search_type || "web");
  const dataState = text(supplied.data_state || expected.data_state || "final");
  const sourceStatus = text(inputs.source_status) === "provider_readback"
    ? "provider_readback"
    : "supplied_result";

  if (!validProperty(property)) {
    findings.push(finding("gsc.property.invalid", "property must be an HTTP(S) URL-prefix or sc-domain property"));
  }
  if (!date(startDate) || !date(endDate) || startDate > endDate) {
    findings.push(finding("gsc.date_range.invalid", "start_date and end_date must form an ordered YYYY-MM-DD range"));
  }
  if (dimensions.length === 0 || dimensions.some((dimension) => !ALLOWED_DIMENSIONS.has(dimension))) {
    findings.push(finding("gsc.dimensions.invalid", "dimensions must be a non-empty supported ordered subset"));
  }
  if (new Set(dimensions).size !== dimensions.length) {
    findings.push(finding("gsc.dimensions.duplicate", "dimensions cannot contain duplicates"));
  }
  if (!ALLOWED_SEARCH_TYPES.has(searchType)) {
    findings.push(finding("gsc.search_type.invalid", "search_type is not supported"));
  }
  if (!ALLOWED_DATA_STATES.has(dataState)) {
    findings.push(finding("gsc.data_state.invalid", "data_state must be final, all, or hourly_all"));
  }
  if (dataState === "hourly_all" && !dimensions.includes("hour")) {
    findings.push(finding(
      "gsc.hourly_all.hour_dimension_missing",
      "hourly_all data requires the hour dimension",
    ));
  }
  if (dimensions.includes("hour") && dataState !== "hourly_all") {
    findings.push(finding(
      "gsc.hour.data_state_mismatch",
      "the hour dimension requires hourly_all data_state",
    ));
  }

  for (const field of ["property", "start_date", "end_date", "search_type", "data_state"]) {
    const expectedValue = text(expected[field]);
    const suppliedValue = text(supplied[field]);
    if (expectedValue && suppliedValue && expectedValue !== suppliedValue) {
      findings.push(finding(`gsc.request.${field}_mismatch`, `supplied ${field} does not match the request`));
    }
  }
  const expectedDimensions = stringArray(expected.dimensions);
  if (
    expectedDimensions.length > 0
    && dimensions.join("\u0000") !== expectedDimensions.join("\u0000")
  ) {
    findings.push(finding("gsc.request.dimensions_mismatch", "supplied dimensions do not match the request"));
  }

  const rawRows = Array.isArray(supplied.rows) ? supplied.rows : [];
  if (rawRows.length > 25000) {
    findings.push(finding("gsc.rows.too_many", "one evidence packet cannot contain more than 25000 rows"));
  }
  const rows = rawRows.slice(0, 25000).map((row, index) =>
    normalizePerformanceRow(row, dimensions, index, findings)
  );

  const metadata = object(supplied.metadata);
  const firstIncompleteDate = text(metadata.first_incomplete_date);
  const firstIncompleteHour = text(metadata.first_incomplete_hour);
  if (firstIncompleteDate && !date(firstIncompleteDate)) {
    findings.push(finding("gsc.metadata.first_incomplete_date_invalid", "first_incomplete_date must be YYYY-MM-DD"));
  }
  if (firstIncompleteHour && !offsetHour(firstIncompleteHour)) {
    findings.push(finding(
      "gsc.metadata.first_incomplete_hour_invalid",
      "first_incomplete_hour must be an ISO-8601 offset hour",
    ));
  }
  const complete = !firstIncompleteDate && !firstIncompleteHour;
  const caveats = [];
  if (!complete) {
    caveats.push("Provider metadata marks part of this period as incomplete.");
  }
  if (dataState !== "final") {
    caveats.push("The request admitted non-final Search Console data.");
  }

  const paginationInput = object(supplied.pagination);
  const returnedRows = integerOr(paginationInput.returned_rows, rows.length);
  const reportedRowCount = integerOr(supplied.row_count, rows.length);
  const paginationComplete = typeof paginationInput.complete === "boolean"
    ? paginationInput.complete
    : returnedRows === 0 || returnedRows < integerOr(expected.row_limit, returnedRows);
  if (!paginationComplete) {
    caveats.push("The packet is a bounded page and does not claim complete property coverage.");
  }

  return {
    performance_draft: {
      schema: "runx.search.performance.evidence.v1",
      decision: findings.length > 0
        ? "blocked"
        : caveats.length > 0
          ? "usable_with_caveats"
          : "ready",
      provider: "google-search-console",
      provider_status: sourceStatus === "provider_readback" ? "readback_verified" : "not_called",
      source_status: sourceStatus,
      property,
      request: {
        start_date: startDate,
        end_date: endDate,
        dimensions,
        search_type: searchType,
        data_state: dataState,
      },
      rows,
      row_count: reportedRowCount,
      pagination: {
        returned_rows: returnedRows,
        complete: paginationComplete,
        next_start_row: nonNegativeIntegerOrNull(paginationInput.next_start_row),
      },
      aggregation_type: text(supplied.aggregation_type),
      freshness: {
        complete,
        data_state: dataState,
        first_incomplete_date: firstIncompleteDate,
        first_incomplete_hour: firstIncompleteHour,
        fetched_at: text(supplied.fetched_at),
      },
      caveats,
      validation: {
        status: findings.length === 0 ? "pass" : "fail",
        findings,
      },
    },
  };
}

export function finalizePerformance(inputs) {
  const draft = object(inputs.performance_draft);
  return {
    performance_evidence: {
      ...draft,
      evidence_digest: digest(inputs.digest_result),
    },
  };
}

export function normalizeInspection(inputs) {
  const supplied = object(inputs.provider_result);
  const expected = object(inputs.request);
  const findings = [];
  const property = text(supplied.property || expected.property);
  const inspectionUrl = text(supplied.inspection_url || expected.inspection_url);

  if (!validProperty(property)) {
    findings.push(finding("gsc.property.invalid", "property must be an HTTP(S) URL-prefix or sc-domain property"));
  }
  if (!webUrl(inspectionUrl)) {
    findings.push(finding("gsc.inspection_url.invalid", "inspection_url must be an absolute HTTP(S) URL"));
  }
  if (property && inspectionUrl && !propertyCovers(property, inspectionUrl)) {
    findings.push(finding("gsc.inspection_url.outside_property", "inspection_url is not covered by the property"));
  }
  for (const field of ["property", "inspection_url"]) {
    if (text(expected[field]) && text(supplied[field]) && text(expected[field]) !== text(supplied[field])) {
      findings.push(finding(`gsc.inspection.${field}_mismatch`, `supplied ${field} does not match the request`));
    }
  }

  return {
    inspection_draft: {
      schema: "runx.search.url_inspection.evidence.v1",
      decision: findings.length === 0 ? "ready" : "blocked",
      provider: "google-search-console",
      provider_status: "readback_verified",
      property,
      inspection_url: inspectionUrl,
      index_status: {
        verdict: text(supplied.verdict),
        coverage_state: text(supplied.coverage_state),
        robots_txt_state: text(supplied.robots_txt_state),
        indexing_state: text(supplied.indexing_state),
        page_fetch_state: text(supplied.page_fetch_state),
        crawled_as: text(supplied.crawled_as),
        last_crawl_time: text(supplied.last_crawl_time),
        referring_urls: stringArray(supplied.referring_urls),
        sitemap: stringArray(supplied.sitemap),
      },
      amp: object(supplied.amp),
      mobile_usability: object(supplied.mobile_usability),
      rich_results: object(supplied.rich_results),
      inspection_link: text(supplied.inspection_link),
      fetched_at: text(supplied.fetched_at),
      validation: {
        status: findings.length === 0 ? "pass" : "fail",
        findings,
      },
    },
  };
}

export function finalizeInspection(inputs) {
  const draft = object(inputs.inspection_draft);
  return {
    inspection_evidence: {
      ...draft,
      evidence_digest: digest(inputs.digest_result),
    },
  };
}

export function prepareSitemapPlan(inputs) {
  const property = text(inputs.property);
  const sitemapUrl = text(inputs.sitemap_url);
  const findings = [];

  if (!validProperty(property)) {
    findings.push(finding("gsc.property.invalid", "property must be an HTTP(S) URL-prefix or sc-domain property"));
  }
  if (!webUrl(sitemapUrl)) {
    findings.push(finding("gsc.sitemap_url.invalid", "sitemap_url must be an absolute HTTP(S) URL"));
  }
  if (property && sitemapUrl && !propertyCovers(property, sitemapUrl)) {
    findings.push(finding("gsc.sitemap.outside_property", "sitemap_url is not covered by the property"));
  }

  const digestSubject = {
    provider: "google-search-console",
    operation: "sitemaps.submit",
    property,
    sitemap_url: sitemapUrl,
  };
  return {
    sitemap_plan_draft: {
      schema: "runx.search.sitemap_plan.v1",
      decision: findings.length === 0 ? "ready_for_approval" : "blocked",
      ...digestSubject,
      provider_status: "not_called",
      external_status: "not_submitted",
      validation: {
        status: findings.length === 0 ? "pass" : "fail",
        findings,
      },
    },
    digest_subject: digestSubject,
  };
}

export function bindSitemapPlan(inputs) {
  return {
    sitemap_plan: {
      ...object(inputs.sitemap_plan_draft),
      plan_digest: digest(inputs.digest_result),
    },
  };
}

export function admitSitemapSubmission(inputs) {
  const plan = object(inputs.sitemap_plan);
  const findings = [];
  const computedDigest = digest(inputs.digest_result);

  if (text(plan.schema) !== "runx.search.sitemap_plan.v1") {
    findings.push(finding("gsc.sitemap_plan.schema_invalid", "sitemap plan schema is not supported"));
  }
  if (text(plan.decision) !== "ready_for_approval") {
    findings.push(finding("gsc.sitemap_plan.not_ready", "sitemap plan is not ready for approval"));
  }
  if (text(plan.provider) !== "google-search-console" || text(plan.operation) !== "sitemaps.submit") {
    findings.push(finding("gsc.sitemap_plan.operation_mismatch", "sitemap plan does not bind the Search Console submit operation"));
  }
  if (text(plan.provider_status) !== "not_called" || text(plan.external_status) !== "not_submitted") {
    findings.push(finding("gsc.sitemap_plan.already_advanced", "sitemap plan claims provider activity"));
  }
  if (text(plan.plan_digest) !== computedDigest) {
    findings.push(finding("gsc.sitemap_plan.digest_mismatch", "sitemap plan fields do not match its native digest"));
  }

  return {
    submission_admission: {
      decision: findings.length === 0 ? "ready" : "blocked",
      property: text(plan.property),
      sitemap_url: text(plan.sitemap_url),
      plan_digest: text(plan.plan_digest),
      validation: {
        status: findings.length === 0 ? "pass" : "fail",
        findings,
      },
    },
  };
}

export function finalizeSitemapSubmission(inputs) {
  const plan = object(inputs.sitemap_plan);
  const mutation = object(inputs.mutation_result);
  const readback = object(inputs.readback_result);
  const findings = [];
  const property = text(plan.property);
  const sitemapUrl = text(plan.sitemap_url);

  for (const [label, result] of [["mutation", mutation], ["readback", readback]]) {
    if (text(result.property) !== property || text(result.sitemap_url) !== sitemapUrl) {
      findings.push(finding(
        `gsc.sitemap_submission.${label}_identity_mismatch`,
        `${label} does not bind the exact property and sitemap URL`,
      ));
    }
  }

  return {
    sitemap_submission: {
      schema: "runx.search.sitemap_submission.v1",
      decision: findings.length === 0 ? "completed" : "blocked",
      provider: "google-search-console",
      operation: "sitemaps.submit",
      property,
      sitemap_url: sitemapUrl,
      plan_digest: text(plan.plan_digest),
      idempotency_key: text(inputs.idempotency_key),
      provider_status: findings.length === 0 ? "readback_verified" : "readback_mismatch",
      external_status: findings.length === 0 ? "submitted" : "unverified",
      mutation: {
        accepted_at: text(mutation.accepted_at),
      },
      readback: {
        status: text(readback.status),
        last_submitted: text(readback.last_submitted),
        last_downloaded: text(readback.last_downloaded),
        error_count: nonNegativeIntegerOrNull(readback.error_count),
        warning_count: nonNegativeIntegerOrNull(readback.warning_count),
      },
      validation: {
        status: findings.length === 0 ? "pass" : "fail",
        findings,
      },
    },
  };
}

export function classifyIndexingRequest(inputs) {
  const url = text(inputs.url);
  const resourceType = text(inputs.resource_type).toLowerCase().replaceAll("-", "_");
  const eligible = new Set(["job_posting", "broadcast_event"]);
  const findings = [];

  if (!webUrl(url)) {
    findings.push(finding("gsc.indexing.url_invalid", "url must be an absolute HTTP(S) URL"));
  }

  const specialist = findings.length === 0 && eligible.has(resourceType);
  return {
    indexing_admission: {
      schema: "runx.search.indexing_admission.v1",
      decision: findings.length > 0 ? "blocked" : specialist ? "specialist_required" : "refused",
      reason_code: findings.length > 0
        ? "invalid_request"
        : specialist
          ? "restricted_api_specialist_review"
          : "unsupported_resource_type",
      url,
      resource_type: resourceType,
      operator_reason: text(inputs.reason),
      provider_status: "not_called",
      external_status: "not_requested",
      downstream_handoff: specialist
        ? {
            state: "specialist_review_required",
            expected_outcome: "confirm Google Indexing API eligibility and use a separately governed implementation",
          }
        : {},
      validation: {
        status: findings.length === 0 ? "pass" : "fail",
        findings,
      },
    },
  };
}

function normalizePerformanceRow(value, dimensions, index, findings) {
  const row = object(value);
  const keys = Array.isArray(row.keys) ? row.keys.map((item) => text(item)) : [];
  const named = object(row.dimensions);
  const mapped = {};

  if (keys.length > 0 && keys.length !== dimensions.length) {
    findings.push(finding(
      "gsc.row.dimension_count_mismatch",
      `row ${index} has ${keys.length} keys for ${dimensions.length} dimensions`,
    ));
  }
  dimensions.forEach((dimension, dimensionIndex) => {
    mapped[dimension] = keys.length > 0 ? text(keys[dimensionIndex]) : text(named[dimension]);
    if (!mapped[dimension]) {
      findings.push(finding("gsc.row.dimension_missing", `row ${index} is missing dimension ${dimension}`));
    }
  });

  const metrics = {};
  for (const field of ["clicks", "impressions", "ctr", "position"]) {
    const numeric = numberOrNull(row[field]);
    if (numeric === null || numeric < 0) {
      findings.push(finding("gsc.row.metric_invalid", `row ${index} has invalid ${field}`));
      metrics[field] = 0;
    } else {
      metrics[field] = numeric;
    }
  }
  if (metrics.ctr > 1) {
    findings.push(finding("gsc.row.ctr_invalid", `row ${index} CTR must be between 0 and 1`));
  }

  return { dimensions: mapped, metrics };
}

function propertyCovers(property, candidate) {
  try {
    const url = new URL(candidate);
    if (property.startsWith("sc-domain:")) {
      const domain = property.slice("sc-domain:".length).toLowerCase();
      const host = url.hostname.toLowerCase();
      return host === domain || host.endsWith(`.${domain}`);
    }
    return url.href.startsWith(property);
  } catch {
    return false;
  }
}

function validProperty(value) {
  if (value.startsWith("sc-domain:")) {
    return /^[a-z0-9.-]+$/u.test(value.slice("sc-domain:".length))
      && !value.endsWith(".")
      && !value.includes("..");
  }
  return webUrl(value);
}

function webUrl(value) {
  try {
    const url = new URL(value);
    return new Set(["http:", "https:"]).has(url.protocol) && Boolean(url.hostname);
  } catch {
    return false;
  }
}

function date(value) {
  return /^\d{4}-\d{2}-\d{2}$/u.test(value);
}

function offsetHour(value) {
  return /^\d{4}-\d{2}-\d{2}T\d{2}:00:00(?:Z|[+-]\d{2}:\d{2})$/u.test(value);
}

function digest(value) {
  const candidate = text(object(value).digest);
  return /^sha256:[0-9a-f]{64}$/u.test(candidate) ? candidate : "";
}

function finding(code, message) {
  return { code, message };
}

function object(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function stringArray(value) {
  return Array.isArray(value) ? value.map((item) => text(item)).filter(Boolean) : [];
}

function numberOrNull(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function integerOr(value, fallback) {
  return Number.isInteger(value) && value >= 0 ? value : fallback;
}

function nonNegativeIntegerOrNull(value) {
  return Number.isInteger(value) && value >= 0 ? value : null;
}
