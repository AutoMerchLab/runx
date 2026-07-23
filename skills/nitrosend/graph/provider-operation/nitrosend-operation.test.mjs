import assert from "node:assert/strict";
import test from "node:test";

import {
  blockedOperation,
  normalizeOperation,
  prepareOperation,
} from "./nitrosend-operation.mjs";

function mcpPayload(result) {
  return {
    jsonrpc: "2.0",
    id: "fixture",
    result: {
      content: [{
        type: "text",
        text: JSON.stringify({ meta: { tool: "fixture" }, result }),
      }],
    },
  };
}

test("prepares a bounded read through native authenticated HTTP", () => {
  const { operation_plan: plan } = prepareOperation({
    mode: "read",
    operation: "status",
    arguments: {},
    brand_sid: "br_fixture",
  });

  assert.equal(plan.decision, "ready");
  assert.equal(plan.tool, "nitro_get_status");
  assert.deepEqual(plan.allowed_hosts, ["api.nitrosend.com"]);
  assert.deepEqual(plan.auth, { type: "bearer", secret_env: "NITROSEND_API_KEY" });
  assert.equal(plan.requests.length, 1);
  assert.match(plan.requests[0].id, /^[A-Za-z0-9_-]+$/u);
  assert.equal(plan.requests[0].body.id, plan.requests[0].id);
  assert.equal(plan.requests[0].body.params.name, "nitro_get_status");
  assert.equal(plan.requests[0].headers["x-brand-sid"], "br_fixture");
  assert.equal(JSON.stringify(plan).includes("nskey_"), false);
});

test("blocks malformed arguments and non-positive provider ids before HTTP", () => {
  const malformed = prepareOperation({ mode: "read", operation: "status", arguments: [] }).operation_plan;
  assert.equal(malformed.decision, "needs_input");
  assert.deepEqual(malformed.requests, []);

  for (const import_id of ["", 0, -1]) {
    const plan = prepareOperation({
      mode: "read",
      operation: "import_status",
      arguments: { import_id },
    }).operation_plan;
    assert.equal(plan.decision, "needs_input");
    assert.deepEqual(plan.requests, []);
  }
});

test("maps consented inline imports without forwarding audit-only fields", () => {
  const { operation_plan: plan } = prepareOperation({
    mode: "act",
    operation: "import_contacts",
    arguments: {
      source_id: "product-signup",
      consent_basis: "First-party signup opt-in",
      records: [{ email: "fixture@example.com" }],
      dry_run: true,
      idempotency_key: "fixture-import",
    },
  });

  const args = plan.requests[0].body.params.arguments;
  assert.equal(args.source_id, undefined);
  assert.equal(args.consent_basis, undefined);
  assert.equal(args.records[0].source, "product-signup");
});

test("normalizes provider readback and redacts provider-returned secrets", () => {
  const returnedSecret = ["nskey", "live", "secret"].join("_");
  const plan = prepareOperation({ mode: "read", operation: "status", arguments: {} }).operation_plan;
  const { provider_evidence: evidence } = normalizeOperation({
    operation_plan: plan,
    http_execution: {
      responses: [{
        id: "nitrosend:status",
        status: 200,
        ok: true,
        body_digest: "sha256:fixture",
        json: mcpPayload({ data: { id: 42, api_token: returnedSecret } }),
      }],
    },
  });

  assert.equal(evidence.decision, "ok");
  assert.equal(evidence.provider_ref, "nitrosend:status:42");
  assert.equal(evidence.result.data.api_token, "[REDACTED]");
  assert.equal(evidence.evidence.body_digest, "sha256:fixture");
  assert.equal(JSON.stringify(evidence).includes(returnedSecret), false);
});

test("projects credential rejection and local validation as bounded evidence", () => {
  const plan = prepareOperation({ mode: "read", operation: "status", arguments: {} }).operation_plan;
  const rejected = normalizeOperation({
    operation_plan: plan,
    http_execution: {
      responses: [{ id: "nitrosend:status", status: 401, ok: false, body_digest: "sha256:401" }],
    },
  }).provider_evidence;
  assert.equal(rejected.decision, "needs_input");
  assert.match(rejected.blockers[0], /credential/u);

  const blockedPlan = prepareOperation({ mode: "act", operation: "unknown", arguments: {} }).operation_plan;
  const blocked = blockedOperation({ operation_plan: blockedPlan }).provider_evidence;
  assert.equal(blocked.decision, "needs_input");
  assert.equal(blocked.evidence, null);
});

test("admits only non-persisting campaign composition reads", () => {
  const intent = prepareOperation({
    mode: "read",
    operation: "compose_campaign_intent",
    arguments: { composition_mode: "intent", goal: "Write a product update" },
  }).operation_plan;
  assert.equal(intent.decision, "ready");
  assert.equal(intent.tool, "nitro_compose_campaign");
  assert.deepEqual(intent.requests[0].body.params.arguments, {
    composition_mode: "intent",
    goal: "Write a product update",
    dry_run: true,
  });

  const validate = prepareOperation({
    mode: "read",
    operation: "validate_campaign_composition",
    arguments: {
      composition_mode: "validate",
      contract_id: "ecc_fixture",
      subject: "A careful update",
      body: "We changed one detail because customers showed us where it hurt.",
    },
  }).operation_plan;
  assert.equal(validate.decision, "ready");
  assert.equal(validate.tool, "nitro_compose_campaign");
  assert.deepEqual(validate.requests[0].body.params.arguments, {
    composition_mode: "validate",
    contract_id: "ecc_fixture",
    subject: "A careful update",
    body: "We changed one detail because customers showed us where it hurt.",
    validate_only: true,
    dry_run: true,
  });
});

test("refuses persistence and delivery fields on campaign composition reads", () => {
  const cases = [
    ["compose_campaign_intent", { composition_mode: "draft", goal: "No" }],
    ["compose_campaign_intent", { composition_mode: "intent", audience: { audience_type: "all_contacts" } }],
    ["validate_campaign_composition", { composition_mode: "validate", contract_id: "ecc_fixture", body: "Hi", scheduled_at: "2026-08-01T00:00:00Z" }],
    ["validate_campaign_composition", { composition_mode: "draft", contract_id: "ecc_fixture", body: "Hi" }],
    ["validate_campaign_composition", { composition_mode: "validate", body: "Hi" }],
  ];

  for (const [operation, args] of cases) {
    const plan = prepareOperation({ mode: "read", operation, arguments: args }).operation_plan;
    assert.notEqual(plan.decision, "ready", `${operation} unexpectedly admitted ${JSON.stringify(args)}`);
    assert.deepEqual(plan.requests, []);
  }
});
