export default function validatePlan(inputs) {
  const principal = requiredString(inputs.principal, "principal");
  const channel = requiredString(inputs.channel, "channel");
  const source = requiredRecord(inputs.source, "source");
  const redaction = requiredRecord(inputs.redaction, "redaction");
  const approval = requiredRecord(inputs.approval, "approval");
  const sendPlan = requiredRecord(inputs.send_plan, "send_plan");
  if (source.decision !== "ready") throw new Error("source provider readback must be ready");
  if (redaction.decision !== "ready") throw new Error("redaction must be ready");
  if (approval.approved !== true) throw new Error("the exact outbound plan must be approved");
  if (sendPlan.decision !== "ready") throw new Error("send plan must be ready");
  assertEqual(requiredString(record(sendPlan.principal).ref, "send_plan.principal.ref"), principal, "send plan principal differs from the approved principal");
  assertEqual(requiredString(record(sendPlan.audience).ref, "send_plan.audience.ref"), channel, "send plan audience differs from the approved audience");
  const redactedDigest = requiredDigest(redaction.redacted_digest, "redaction.redacted_digest");
  assertEqual(requiredDigest(record(sendPlan.content).digest, "send_plan.content.digest"), redactedDigest, "send plan digest differs from the scrubbed artifact");
  if (record(sendPlan.success_checkpoint).milestone !== "provider_delivery_required") {
    throw new Error("send plan must leave provider delivery and readback outstanding");
  }
  if (sendPlan.delivery_evidence || sendPlan.provider_receipt || sendPlan.delivery_status === "delivered") {
    throw new Error("plan-only handoff must not claim provider delivery evidence");
  }
  return {
    outbound_plan: {
      decision: "ready",
      completion: "plan_only",
      provider_delivery: "not_executed",
      principal,
      audience: channel,
      source: {
        final_url: requiredString(source.final_url, "source.final_url"),
        digest: requiredDigest(source.content_digest, "source.content_digest"),
        fetched_at: requiredString(record(source.provenance).fetched_at, "source.provenance.fetched_at"),
      },
      redaction: {
        digest: redactedDigest,
        source_digest: requiredDigest(redaction.source_digest, "redaction.source_digest"),
      },
      approval: { approved: true, gate_id: optionalString(approval.gate_id) || "governed-outbound.send.approval" },
      provider_actions: strings(sendPlan.provider_actions),
      send_plan: sendPlan,
    },
  };
}

export function bindDelivery(inputs) {
  const redaction = requiredRecord(inputs.redaction, "redaction");
  if (redaction.decision !== "ready") throw new Error("redaction must be ready before delivery binding");
  const redactedContent = requiredString(redaction.redacted_content, "redaction.redacted_content");
  const redactedDigest = requiredDigest(redaction.redacted_digest, "redaction.redacted_digest");
  const candidate = requiredRecord(inputs.delivery, "delivery");
  const payloadTemplate = requiredRecord(candidate.payload, "delivery.payload");
  const replacement = replacePlaceholder(payloadTemplate, redactedContent);
  if (replacement.count !== 1) {
    throw new Error("delivery.payload must contain exactly one {{runx.redacted_content}} placeholder");
  }
  const resultFields = strings(candidate.result_fields);
  if (!resultFields.includes("content_digest")) {
    throw new Error("delivery.result_fields must include content_digest for independent readback");
  }
  return {
    delivery: {
      provider: requiredString(candidate.provider, "delivery.provider"),
      target: requiredString(candidate.target, "delivery.target"),
      operation: requiredString(candidate.operation, "delivery.operation"),
      readback_operation: requiredString(candidate.readback_operation, "delivery.readback_operation"),
      payload: replacement.value,
      expected_result: { ...record(candidate.expected_result), content_digest: redactedDigest },
      result_fields: resultFields,
    },
  };
}

function replacePlaceholder(value, content) {
  if (value === "{{runx.redacted_content}}") return { value: content, count: 1 };
  if (Array.isArray(value)) {
    let count = 0;
    const output = value.map((entry) => {
      const replaced = replacePlaceholder(entry, content);
      count += replaced.count;
      return replaced.value;
    });
    return { value: output, count };
  }
  if (value && typeof value === "object") {
    let count = 0;
    const output = Object.fromEntries(Object.entries(value).map(([key, entry]) => {
      const replaced = replacePlaceholder(entry, content);
      count += replaced.count;
      return [key, replaced.value];
    }));
    return { value: output, count };
  }
  return { value, count: 0 };
}

function assertEqual(actual, expected, message) { if (actual !== expected) throw new Error(message); }
function requiredDigest(value, field) {
  const parsed = requiredString(value, field);
  if (!/^sha256:[0-9a-f]{64}$/u.test(parsed)) throw new Error(`${field} must be a sha256 digest`);
  return parsed;
}
function requiredString(value, field) {
  const parsed = optionalString(value);
  if (!parsed) throw new Error(`${field} must be a non-empty string`);
  return parsed;
}
function optionalString(value) { return typeof value === "string" && value.trim() ? value.trim() : null; }
function strings(value) { return Array.isArray(value) ? [...new Set(value.map(optionalString).filter(Boolean))] : []; }
function record(value) { return value && typeof value === "object" && !Array.isArray(value) ? value : {}; }
function requiredRecord(value, field) {
  const parsed = record(value);
  if (Object.keys(parsed).length === 0) throw new Error(`${field} must be a non-empty object`);
  return parsed;
}
