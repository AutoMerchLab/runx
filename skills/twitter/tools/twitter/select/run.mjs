#!/usr/bin/env node

// Deterministic bulk selector. Reads an X archive export, applies a typed
// predicate, and emits a compact twitter.plan.v1 of delete acts, no LLM. This
// is the bulk lane: mechanical criteria over thousands of posts, where a
// per-item agent rationale would be wrong and would exceed the runtime output
// cap. Curated, judgment-bearing pruning still goes through the `plan` runner.

import fs from "node:fs";
import {
  canonicalDigest,
  fail,
  readInputs,
  resolveSkillPath,
  writePacket,
} from "../lib/client.mjs";

function packet(overrides) {
  return {
    decision: "ready",
    objective: "",
    principal: "",
    source: "archive",
    predicate: {},
    matched: 0,
    scanned: 0,
    truncated: false,
    twitter_plan: null,
    plan_digest: "",
    blockers: [],
    ...overrides,
  };
}

function parseArchive(filePath) {
  const raw = fs.readFileSync(filePath, "utf8");
  const eq = raw.indexOf("=");
  const body = eq >= 0 ? raw.slice(eq + 1) : raw;
  const entries = JSON.parse(body.trim());
  if (!Array.isArray(entries)) throw new Error("archive file did not contain an array");
  return entries.map((entry) => entry.tweet ?? entry);
}

function retweetAuthor(text) {
  const match = (text || "").match(/^RT @([A-Za-z0-9_]+):/);
  return match ? match[1] : null;
}

function toYear(createdAt) {
  const year = String(createdAt || "").slice(-4);
  return /^\d{4}$/.test(year) ? Number(year) : null;
}

// Each predicate field is optional; a post must satisfy every provided field.
function matches(tweet, predicate) {
  const text = tweet.full_text ?? tweet.text ?? "";
  const author = retweetAuthor(text);
  if (predicate.is_retweet === true && !author) return false;
  if (predicate.is_retweet === false && author) return false;
  if (predicate.rt_of && (author ?? "").toLowerCase() !== String(predicate.rt_of).toLowerCase()) return false;
  if (predicate.text_prefix && !text.startsWith(predicate.text_prefix)) return false;
  if (predicate.text_contains && !text.toLowerCase().includes(String(predicate.text_contains).toLowerCase())) return false;
  const likes = Number(tweet.favorite_count ?? 0);
  const reposts = Number(tweet.retweet_count ?? 0);
  if (predicate.max_likes !== undefined && likes > Number(predicate.max_likes)) return false;
  if (predicate.max_reposts !== undefined && reposts > Number(predicate.max_reposts)) return false;
  const year = toYear(tweet.created_at);
  if (predicate.before_year !== undefined && !(year !== null && year < Number(predicate.before_year))) return false;
  if (predicate.after_year !== undefined && !(year !== null && year > Number(predicate.after_year))) return false;
  return true;
}

function main() {
  const inputs = readInputs();
  const objective = typeof inputs.objective === "string" ? inputs.objective : "";
  const principal = typeof inputs.principal === "string" ? inputs.principal : "";
  const predicate = typeof inputs.predicate === "object" && inputs.predicate !== null ? inputs.predicate : {};
  const maxActs = Number.isFinite(Number(inputs.max_acts)) && Number(inputs.max_acts) > 0
    ? Math.floor(Number(inputs.max_acts))
    : 5000;

  const blockers = [];
  if (!objective) blockers.push("objective is required");
  if (!principal) blockers.push("principal is required");
  if (!inputs.archive_file) blockers.push("archive_file is required for the select lane");
  if (Object.keys(predicate).length === 0) {
    blockers.push("predicate is required; refusing to select every post by default");
  }
  if (blockers.length > 0) {
    writePacket(packet({ decision: "needs_input", objective, principal, predicate, blockers }));
    return;
  }

  const filePath = resolveSkillPath(String(inputs.archive_file));
  if (!filePath) {
    writePacket(packet({
      decision: "needs_input",
      objective,
      principal,
      predicate,
      blockers: [`archive_file ${inputs.archive_file} was not found`],
    }));
    return;
  }

  const tweets = parseArchive(filePath);
  const acts = [];
  const rationale = `Matched the operator predicate ${JSON.stringify(predicate)}.`;
  for (const tweet of tweets) {
    if (acts.length >= maxActs) break;
    if (!matches(tweet, predicate)) continue;
    const id = tweet.id_str ?? tweet.id;
    if (!id) continue;
    acts.push({
      act_id: `del-${id}`,
      kind: "delete_post",
      params: { post_id: String(id) },
      consequence: "live_mutation",
      rationale,
    });
  }

  // The plan object below is what a driver hands to `execute` as plan_json;
  // its canonical digest is bound here and re-checked at execution, so the
  // approved bytes and the executed bytes are provably identical.
  const plan = {
    decision: "ready",
    objective,
    principal,
    acts,
    gates: { human_approval_required: true, approval_ref: "approval:pending" },
    evidence_refs: [`archive:${inputs.archive_file}`],
    open_questions: [],
    blockers: [],
    success_checkpoint: {
      milestone: "bulk_plan_ready_for_approval",
      description: `${acts.length} delete acts selected by predicate, ready for one approval and staged execution.`,
    },
  };

  writePacket(packet({
    objective,
    principal,
    predicate,
    matched: acts.length,
    scanned: tweets.length,
    truncated: acts.length >= maxActs,
    twitter_plan: plan,
    plan_digest: canonicalDigest(plan),
  }));
}

try {
  main();
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}
