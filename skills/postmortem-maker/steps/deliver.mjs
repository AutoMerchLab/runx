// postmortem-maker / step 4 of 5: execute the authorized send.
//
// This is the provider adapter. It performs the delivery the send_plan
// authorized: it appends the postmortem to the outbox stream under
// compare-and-set, assigns a provider message id, and seals the delivery
// evidence. It runs only when step 3 authorized publication (`when:
// compose.publishable == true`), so a withheld postmortem never reaches this
// file at all.
//
// WHAT THIS TRANSPORT IS, STATED PLAINLY: a bundled append-only outbox log, not
// a hosted provider. The canonical `runx/send-as` skill describes itself as a
// planning and authority layer that "never delivers", and the runtime's native
// `data.*` tools are not in the execution closure of a registry-installed
// package, so this package ships its own adapter. Nothing here claims a hosted
// provider delivered anything. What it does claim is exactly what it does:
//
//   * compare-and-set: the append is refused unless the outbox is still at the
//     version step 2 read, so a concurrent publisher cannot be clobbered,
//   * idempotency: republishing the same postmortem returns the original
//     delivery instead of duplicating it; the same key with different content
//     is refused,
//   * durability: the message is written to disk and survives the run, which is
//     what lets step 5 (and any later run) read it back independently.

import { createHash } from "node:crypto";
import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const PROVIDER = "bundled-local-outbox";

function refuse(reason) {
  console.error(reason);
  process.exit(1);
}

function seal(data) {
  console.log(JSON.stringify(data, null, 2));
  process.exit(0);
}

function parseInput() {
  const raw = process.env.RUNX_INPUTS_PATH
    ? readFileSync(process.env.RUNX_INPUTS_PATH, "utf8")
    : process.env.RUNX_INPUTS_JSON;
  if (!raw) return refuse("No input provided via RUNX_INPUTS_PATH or RUNX_INPUTS_JSON");
  try {
    return JSON.parse(raw);
  } catch (e) {
    return refuse("Invalid JSON input");
  }
}

function asObject(v) {
  if (v && typeof v === "object" && !Array.isArray(v)) return v;
  if (typeof v === "string" && v.trim()) {
    try {
      const p = JSON.parse(v);
      return p && typeof p === "object" && !Array.isArray(p) ? p : {};
    } catch (e) {
      return {};
    }
  }
  return {};
}

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value === undefined ? null : value);
}

function digestOf(value) {
  return `sha256:${createHash("sha256").update(canonical(value)).digest("hex")}`;
}

// Must match steps/read_outbox.mjs and steps/verify.mjs exactly.
function outboxPath(target) {
  const key = `${target.data_source_ref}|${target.channel}|${target.aggregate_id}`;
  const id = createHash("sha256").update(key).digest("hex").slice(0, 16);
  const root = process.env.RUNX_CWD || process.cwd();
  return resolve(join(root, ".postmortem-outbox", `${id}.jsonl`));
}

function readOutbox(path) {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((l) => l.trim().length > 0)
    .map((l) => JSON.parse(l));
}

function pick(input, ...names) {
  for (const n of names) {
    const direct = input[n];
    if (direct && typeof direct === "object") return direct;
    const ctx = input.context && input.context[n];
    if (ctx && typeof ctx === "object") return ctx;
  }
  return null;
}

function scalar(input, name) {
  if (input[name] !== undefined) return input[name];
  if (input.context && input.context[name] !== undefined) return input.context[name];
  return undefined;
}

function main() {
  const input = parseInput();
  const target = asObject(input.publish_target);
  for (const f of ["data_source_ref", "channel", "aggregate_id"]) {
    if (typeof target[f] !== "string" || target[f].length === 0) {
      return refuse(`publish_target.${f} is required`);
    }
  }

  const send_plan = pick(input, "send_plan");
  const message = pick(input, "message");
  if (!send_plan) return refuse("send_plan is required from the compose step");
  if (!message) return refuse("message is required from the compose step");
  if (send_plan.status !== "authorized") {
    return refuse(`send_plan status is "${send_plan.status}"; only an authorized plan may be delivered`);
  }

  const idempotency_key = scalar(input, "idempotency_key");
  if (typeof idempotency_key !== "string" || idempotency_key.length === 0) {
    return refuse("idempotency_key is required from the compose step");
  }
  const expected_version = scalar(input, "expected_version");
  if (typeof expected_version !== "number") {
    return refuse("expected_version is required from the compose step");
  }

  const content_digest = digestOf(message.body);
  if (send_plan.content_digest !== content_digest) {
    return refuse(
      `content digest mismatch: the send plan authorized ${send_plan.content_digest} but the message body digests to ${content_digest}`
    );
  }

  const path = outboxPath(target);
  let records = [];
  try {
    records = readOutbox(path);
  } catch (e) {
    return refuse(`outbox at ${path} is corrupt: unreadable line`);
  }
  const before_version = records.length;

  // Idempotent replay: the same postmortem re-published returns the original
  // delivery rather than sending twice.
  const prior = records.find((r) => r.idempotency_key === idempotency_key);
  if (prior) {
    if (prior.content_digest !== content_digest) {
      return refuse(`idempotency key ${idempotency_key} was reused with different postmortem content`);
    }
    return seal({
      delivery_result: {
        status: "delivered",
        replayed: true,
        provider: PROVIDER,
        operation: "send",
        transport: "bundled append-only outbox log (not a hosted provider)",
        data_source_ref: target.data_source_ref,
        channel: target.channel,
        aggregate_id: target.aggregate_id,
        message_id: prior.message_id,
        message_ref: prior.message_ref,
        content_digest: prior.content_digest,
        before_version: prior.version - 1,
        after_version: prior.version,
        delivered_at: prior.delivered_at,
        idempotency_key,
        store_path: path,
        send_plan_status: send_plan.status,
      },
    });
  }

  if (expected_version !== before_version) {
    return refuse(
      `compare-and-set failed: expected outbox version ${expected_version}, stream is at ${before_version}`
    );
  }

  const after_version = before_version + 1;
  const message_id = createHash("sha256")
    .update(`${target.data_source_ref}|${target.channel}|${target.aggregate_id}|${idempotency_key}`)
    .digest("hex")
    .slice(0, 24);
  const message_ref = `${target.channel}:${target.aggregate_id}:${after_version}`;
  const delivered_at = new Date().toISOString();

  const record = {
    version: after_version,
    message_id,
    message_ref,
    content_digest,
    idempotency_key,
    delivered_at,
    incident_ref: message.incident_ref || null,
    title: message.title || null,
    send_plan,
    body: message.body,
  };

  mkdirSync(dirname(path), { recursive: true });
  appendFileSync(path, `${JSON.stringify(record)}\n`, "utf8");

  seal({
    delivery_result: {
      status: "delivered",
      replayed: false,
      provider: PROVIDER,
      operation: "send",
      transport: "bundled append-only outbox log (not a hosted provider)",
      data_source_ref: target.data_source_ref,
      channel: target.channel,
      aggregate_id: target.aggregate_id,
      message_id,
      message_ref,
      content_digest,
      before_version,
      after_version,
      delivered_at,
      idempotency_key,
      store_path: path,
      send_plan_status: send_plan.status,
    },
  });
}

main();
