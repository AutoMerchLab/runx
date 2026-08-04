// crm-cleanup / step 3 of 3: execute the decided updates.
//
// WHAT THIS IS, STATED PLAINLY: a bounded append-only event log bundled with
// the skill, not the runx data-store. The runtime's native `data.*` tools
// (data.read_projection / data.append_event / data.source) are reachable from a
// local project run but are NOT in the execution closure of a package installed
// from the registry — a registry run fails with
//   "Imported tool 'data.read_projection' was not found in configured tool catalogs"
// so a published skill cannot call them. This transport implements the same
// contract the data store enforces, and the receipt records exactly that:
//
//   * compare-and-set: the append is refused unless the stream is still at the
//     version step 1 read, so a concurrent writer cannot be clobbered,
//   * idempotency: replaying the same key with the same content returns the
//     original commit; the same key with different content is refused,
//   * a sealed before/after: before_version -> after_version, event_ref, and a
//     sha256 event digest over the canonical event bytes.
//
// It writes one JSON line per event under the run's working directory. Nothing
// here claims to be a governed data-store append.

import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const PROVIDER = "bundled-local-event-log";

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

// Canonical JSON so the digest is stable regardless of key order.
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

// One log file per (data_source_ref, resource, aggregate_id), addressed by hash
// so a ref like "local://runx-crm/dogfood" never escapes the working directory.
function storePath(handle) {
  const key = `${handle.data_source_ref}|${handle.resource}|${handle.aggregate_id}`;
  const id = createHash("sha256").update(key).digest("hex").slice(0, 16);
  const root = process.env.RUNX_CWD || process.cwd();
  return resolve(join(root, ".crm-store", `${id}.jsonl`));
}

function readLog(path) {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((l) => l.trim().length > 0)
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch (e) {
        return refuse(`Event log at ${path} is corrupt: unreadable line`);
      }
    });
}

function main() {
  const input = parseInput();
  const handle = input.source_handle && typeof input.source_handle === "object" ? input.source_handle : null;
  if (!handle) return refuse("source_handle is required");
  for (const f of ["data_source_ref", "resource", "aggregate_id"]) {
    if (typeof handle[f] !== "string" || handle[f].length === 0) {
      return refuse(`source_handle.${f} is required`);
    }
  }
  const event = input.event;
  if (!event || typeof event !== "object") return refuse("event is required");
  const idempotency_key = input.idempotency_key;
  if (typeof idempotency_key !== "string" || idempotency_key.length === 0) {
    return refuse("idempotency_key is required");
  }
  if (typeof input.expected_version !== "number") return refuse("expected_version is required");

  const path = storePath(handle);
  const log = readLog(path);
  const before_version = log.length;
  const event_digest = digestOf(event);

  // Idempotent replay: same key, same content -> report the original commit.
  const prior = log.find((e) => e.idempotency_key === idempotency_key);
  if (prior) {
    if (prior.event_digest !== event_digest) {
      return refuse(
        `idempotency key ${idempotency_key} was reused with different event content`
      );
    }
    return seal({
      write_result: {
        status: "committed",
        replayed: true,
        provider: PROVIDER,
        operation: "append_event",
        data_source_ref: handle.data_source_ref,
        resource: handle.resource,
        aggregate_id: handle.aggregate_id,
        before_version: prior.version - 1,
        after_version: prior.version,
        event_ref: prior.event_ref,
        event_digest: prior.event_digest,
        idempotency_key,
        store_path: path,
        before: input.before || {},
        after: (event.payload && event.payload.fields) || {},
      },
    });
  }

  // Compare-and-set against the version step 1 read.
  if (input.expected_version !== before_version) {
    return refuse(
      `compare-and-set failed: expected version ${input.expected_version}, stream is at ${before_version}`
    );
  }

  const after_version = before_version + 1;
  const event_ref = `${handle.resource}:${handle.aggregate_id}:${after_version}`;
  const record = {
    version: after_version,
    event_ref,
    event_digest,
    idempotency_key,
    event,
  };

  mkdirSync(dirname(path), { recursive: true });
  appendFileSync(path, `${JSON.stringify(record)}\n`, "utf8");

  seal({
    write_result: {
      status: "committed",
      replayed: false,
      provider: PROVIDER,
      operation: "append_event",
      data_source_ref: handle.data_source_ref,
      resource: handle.resource,
      aggregate_id: handle.aggregate_id,
      before_version,
      after_version,
      event_ref,
      event_digest,
      idempotency_key,
      store_path: path,
      before: input.before || {},
      after: (event.payload && event.payload.fields) || {},
    },
  });
}

main();
