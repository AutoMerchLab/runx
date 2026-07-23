const API_URL = "https://api.nitrosend.com/mcp";
const API_HOST = "api.nitrosend.com";
const READ_OPERATIONS = new Map([
  ["status", "nitro_get_status"],
  ["insights", "nitro_get_insights"],
  ["review_delivery", "nitro_review_delivery"],
  ["import_status", "nitro_query"],
  ["compose_campaign_intent", "nitro_compose_campaign"],
  ["validate_campaign_composition", "nitro_compose_campaign"],
]);
const ACT_OPERATIONS = new Map([
  ["send_transactional", "nitro_send_message"],
  ["control_delivery", "nitro_control_delivery"],
  ["import_contacts", "nitro_import_contacts"],
  ["compose_campaign", "nitro_compose_campaign"],
  ["compose_flow", "nitro_compose_flow"],
  ["manage_template", "nitro_manage_template"],
  ["define_segment", "nitro_define_segment"],
]);
const DELIVERY_OPERATIONS = new Set([
  "approve", "reject", "live", "schedule", "pause", "resume", "cancel",
  "archive", "restore", "delete",
]);
const SENSITIVE_KEYS = /authorization|api[_-]?key|bearer|credential|secret|token/iu;
const SECRET_VALUE = /\b(?:nskey|wpkey)_(?:live|test)_[A-Za-z0-9_-]+\b/gu;

export function prepareOperation(inputs) {
  const mode = text(inputs.mode);
  const operation = text(inputs.operation);
  const rawArguments = inputs.arguments;
  const args = record(rawArguments);
  const operations = mode === "read" ? READ_OPERATIONS : mode === "act" ? ACT_OPERATIONS : null;
  const blockers = operations
    ? [
        ...(rawArguments !== undefined && !isRecord(rawArguments)
          ? ["arguments must be a JSON object"]
          : []),
        ...validate(mode, operation, args),
      ]
    : ["mode must be read or act"];
  const decision = blockers.some((blocker) => blocker.startsWith("refused:"))
    ? "refused"
    : blockers.length > 0
      ? "needs_input"
      : "ready";
  const tool = operations?.get(operation) ?? null;
  const requestId = `nitrosend-${operation || "unknown"}`;
  return {
    operation_plan: {
      decision,
      provider: "nitrosend",
      mode,
      operation: operation || null,
      tool,
      requests: decision === "ready"
        ? [{
            id: requestId,
            method: "POST",
            url: API_URL,
            headers: {
              accept: "application/json, text/event-stream",
              ...(text(inputs.brand_sid) ? { "x-brand-sid": text(inputs.brand_sid) } : {}),
            },
            body: {
              jsonrpc: "2.0",
              id: requestId,
              method: "tools/call",
              params: { name: tool, arguments: providerArguments(operation, args) },
            },
          }]
        : [],
      allowed_hosts: [API_HOST],
      auth: { type: "bearer", secret_env: "NITROSEND_API_KEY" },
      blockers: blockers.map((blocker) => blocker.replace(/^refused:/u, "")),
    },
  };
}

export function normalizeOperation(inputs) {
  const plan = record(inputs.operation_plan);
  const execution = record(inputs.http_execution);
  const response = array(execution.responses)[0];
  if (!response || typeof response !== "object" || Array.isArray(response)) {
    return { provider_evidence: evidence(plan, "provider_error", null, null, ["Nitrosend returned no HTTP response evidence"]) };
  }
  const status = number(response.status);
  if (response.ok !== true) {
    const authority = status === 401 || status === 403;
    return {
      provider_evidence: evidence(
        plan,
        authority ? "needs_input" : "provider_error",
        response,
        null,
        [authority ? "Nitrosend rejected the configured credential" : `Nitrosend returned HTTP ${status}`],
      ),
    };
  }
  try {
    const result = parseToolContent(providerPayload(response));
    const safeResult = redact(result);
    const providerError = safeResult?.error === true || safeResult?.isError === true;
    return {
      provider_evidence: evidence(
        plan,
        providerError ? "provider_error" : "ok",
        response,
        safeResult,
        providerError ? [safeResult?.message || "Nitrosend rejected the operation"] : [],
      ),
    };
  } catch (error) {
    return {
      provider_evidence: evidence(
        plan,
        "provider_error",
        response,
        null,
        [redactText(error instanceof Error ? error.message : String(error))],
      ),
    };
  }
}

export function blockedOperation(inputs) {
  const plan = record(inputs.operation_plan);
  return {
    provider_evidence: evidence(
      plan,
      text(plan.decision) || "needs_input",
      null,
      null,
      array(plan.blockers).map(String),
    ),
  };
}

function validate(mode, operation, args) {
  const operations = mode === "read" ? READ_OPERATIONS : ACT_OPERATIONS;
  if (!operations.has(operation)) {
    return [`operation must be one of: ${[...operations.keys()].join(", ")}`];
  }
  if (mode === "read" && operation === "insights") {
    const scopes = ["account", "flow", "campaign", "message"];
    if (!scopes.includes(args.scope)) return [`arguments.scope must be one of: ${scopes.join(", ")}`];
    if (args.scope !== "account" && !positiveInteger(args.entity_id)) {
      return [`arguments.entity_id is required for ${args.scope} insights`];
    }
  }
  if (mode === "read" && operation === "review_delivery") {
    if (!["template", "flow", "campaign"].includes(args.target_type) || !positiveInteger(args.target_id)) {
      return ["review_delivery requires a valid target_type and integer target_id"];
    }
  }
  if (mode === "read" && operation === "import_status" && !positiveInteger(args.import_id)) {
    return ["import_status requires arguments.import_id"];
  }
  if (mode === "read" && ["compose_campaign_intent", "validate_campaign_composition"].includes(operation)) {
    const expectedMode = operation === "compose_campaign_intent" ? "intent" : "validate";
    if (args.composition_mode !== expectedMode) {
      return [`${operation} requires arguments.composition_mode=${expectedMode}`];
    }
    const forbidden = [
      "audience", "scheduled_at", "confirm", "idempotency_key", "campaign_id", "mode",
      "approval", "activate", "activation", "send", "operation",
    ].filter((key) => Object.hasOwn(args, key));
    if (forbidden.length > 0) {
      return [`refused:${operation} cannot receive stateful fields: ${forbidden.join(", ")}`];
    }
    if (operation === "compose_campaign_intent" && args.contract_id !== undefined) {
      return ["compose_campaign_intent must not receive arguments.contract_id"];
    }
    if (operation === "validate_campaign_composition" && !text(args.contract_id)) {
      return ["validate_campaign_composition requires arguments.contract_id"];
    }
    if (operation === "validate_campaign_composition" && !text(args.body) && !Array.isArray(args.sections)) {
      return ["validate_campaign_composition requires arguments.body or arguments.sections"];
    }
  }
  if (mode === "act" && operation === "send_transactional") {
    if (!["email", "sms"].includes(args.channel) || !text(args.to)) {
      return ["send_transactional requires channel email or sms and one recipient"];
    }
    if (args.dry_run !== true && !text(args.idempotency_key)) {
      return ["refused:a real transactional send requires arguments.idempotency_key"];
    }
  }
  if (mode === "act" && operation === "control_delivery") {
    if (!["flow", "campaign"].includes(args.target_type) || !positiveInteger(args.target_id) || !DELIVERY_OPERATIONS.has(args.operation)) {
      return ["control_delivery requires a valid target_type, integer target_id, and lifecycle operation"];
    }
    if (args.operation === "schedule" && !text(args.scheduled_at)) {
      return ["scheduled campaign delivery requires arguments.scheduled_at"];
    }
    if (["live", "schedule"].includes(args.operation) && args.target_type === "campaign" && !text(args.idempotency_key)) {
      return ["refused:live or scheduled campaign delivery requires arguments.idempotency_key"];
    }
  }
  if (mode === "act" && operation === "import_contacts") {
    if (!text(args.source_id) || !text(args.consent_basis)) {
      return ["contact imports require arguments.source_id and arguments.consent_basis"];
    }
    if (/purchased|scraped|data\s*broker/iu.test(args.consent_basis)) {
      return ["refused:purchased, scraped, and data-broker contact sources are not permitted"];
    }
    if (args.dry_run !== true && !text(args.idempotency_key)) {
      return ["refused:a real contact import requires arguments.idempotency_key"];
    }
  }
  return [];
}

function providerArguments(operation, args) {
  if (operation === "compose_campaign_intent") {
    return { ...args, composition_mode: "intent", dry_run: true };
  }
  if (operation === "validate_campaign_composition") {
    return { ...args, composition_mode: "validate", validate_only: true, dry_run: true };
  }
  if (operation === "import_status") {
    return { entity: "imports", filters: { id: Number(args.import_id) }, page: 1, per: 1 };
  }
  if (operation !== "import_contacts") return args;
  const { source_id: sourceId, consent_basis: _consentBasis, ...providerArgs } = args;
  if (Array.isArray(providerArgs.records)) {
    providerArgs.records = providerArgs.records.map((entry) => {
      const contact = record(entry);
      return { ...contact, source: contact.source || sourceId };
    });
  }
  return providerArgs;
}

function providerPayload(response) {
  if (response.json && typeof response.json === "object" && !Array.isArray(response.json)) {
    return response.json;
  }
  const body = text(response.body);
  if (!body) throw new Error("Nitrosend returned an empty MCP response");
  if (body.startsWith("{")) return JSON.parse(body);
  const payloads = body
    .split(/\r?\n/u)
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice(5).trim())
    .filter((line) => line && line !== "[DONE]");
  if (payloads.length === 0) throw new Error("Nitrosend returned an invalid MCP event stream");
  return JSON.parse(payloads.at(-1));
}

function parseToolContent(payload) {
  if (payload.error) throw new Error(payload.error.message || "Nitrosend MCP request failed");
  const content = payload.result?.content;
  if (!Array.isArray(content)) return payload.result ?? {};
  const value = content.find((item) => item?.type === "text")?.text;
  if (typeof value !== "string") return payload.result ?? {};
  try {
    const parsed = JSON.parse(value);
    if (parsed && typeof parsed === "object" && parsed.meta?.tool && Object.hasOwn(parsed, "result")) {
      return parsed.result;
    }
    return parsed;
  } catch {
    return { message: value };
  }
}

function evidence(plan, decision, response, result, blockers) {
  return {
    decision,
    provider: "nitrosend",
    mode: text(plan.mode),
    operation: plan.operation ?? null,
    tool: plan.tool ?? null,
    provider_ref: providerReference(text(plan.operation), result),
    result,
    evidence: response
      ? {
          request_id: text(response.id),
          http_status: number(response.status),
          body_digest: text(response.body_digest),
          credential_material: "redacted",
        }
      : null,
    blockers,
  };
}

function providerReference(operation, result) {
  const data = result?.data ?? result;
  const id = data?.id ?? data?.message_id ?? data?.import_id ?? data?.target_id ?? data?.campaign_id ?? data?.flow_id;
  return id === undefined || id === null ? null : `nitrosend:${operation}:${id}`;
}

function redact(value) {
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, child]) => [
      key,
      SENSITIVE_KEYS.test(key) ? "[REDACTED]" : redact(child),
    ]));
  }
  return typeof value === "string" ? redactText(value) : value;
}

function redactText(value) {
  return String(value).replaceAll(SECRET_VALUE, "[REDACTED]").slice(0, 2_000);
}

function record(value) {
  return isRecord(value) ? value : {};
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function array(value) {
  return Array.isArray(value) ? value : [];
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function number(value) {
  return Number.isFinite(Number(value)) ? Number(value) : 0;
}

function positiveInteger(value) {
  return value !== "" && value !== null && value !== undefined && Number.isInteger(Number(value)) && Number(value) > 0;
}
