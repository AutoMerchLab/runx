#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  buildCreateFrame,
  buildLifecycleFrame,
  buildMessageFrame,
} from "../tools/thread/thread_desired_state.mjs";
import {
  firstNonEmptyString,
  listGitHubIssuesWithAnyLabel,
  parseGitHubIssueRef,
  prune,
  readGitHubThreadSnapshot,
} from "../tools/thread/github_adapter.mjs";

// Generic declarative thread-mirror driver. It pulls a tenant's desired thread
// state (provider-agnostic, no domain meaning), reconciles each thread's provider
// representation to match via the governed thread_outbox provider, and reports
// observations back so the tenant can re-link the resulting threads. It is
// stateless and idempotent: every run compares desired vs live and applies only
// the difference, so there is no cursor and drift self-heals on the next tick.

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROVIDER_SCRIPT = path.join(__dirname, "../tools/thread/thread_outbox_provider/github-provider.mjs");

async function main() {
  const config = readConfig(process.env);
  const payload = await fetchDesiredState(config);
  const threads = Array.isArray(payload.threads) ? payload.threads : [];
  const pendingObservations = [];
  const results = [];
  let observed = 0;

  logSyncEvent("start", {
    mode: config.fullReconcile ? "full" : "incremental",
    dry_run: config.dryRun,
    threads: threads.length,
    limit: config.limit,
  });

  for (const [index, thread] of threads.entries()) {
    const startedAt = Date.now();
    try {
      const outcome = reconcileThread(thread, config);
      results.push(outcome.summary);
      if (outcome.observation) {
        pendingObservations.push(outcome.observation);
        observed += await flushObservations(config, pendingObservations);
      }
      logThreadProgress(config, {
        index,
        total: threads.length,
        duration_ms: Date.now() - startedAt,
        ...outcome.summary,
      });
    } catch (error) {
      results.push({ identity_key: thread?.identity_key, error: messageOf(error) });
      logThreadProgress(config, {
        index,
        total: threads.length,
        duration_ms: Date.now() - startedAt,
        identity_key: thread?.identity_key,
        error: messageOf(error),
      });
      if (!config.continueOnError) throw error;
    }
  }

  observed += await flushObservations(config, pendingObservations);
  const orphanResults = retireOrphanManagedThreads({ config, threads, results });
  results.push(...orphanResults);

  await writeJsonLine({
    ok: true,
    reconciled: threads.length,
    orphaned: orphanResults.length,
    observed,
    results,
  });
}

function reconcileThread(thread, config) {
  if (config.dryRun) {
    return {
      summary: {
        identity_key: thread.identity_key,
        would_create: !firstNonEmptyString(thread.thread_locator),
        state: thread.state,
        labels: Array.isArray(thread.labels) ? thread.labels.length : 0,
        comments: Array.isArray(thread.comments) ? thread.comments.length : 0,
        dry_run: true,
      },
    };
  }

  const desiredLabels = Array.isArray(thread.labels) ? thread.labels : [];
  const managedLabels = Array.isArray(thread.managed_labels) ? thread.managed_labels : [];
  const comments = Array.isArray(thread.comments) ? thread.comments : [];

  // 1. Locate or create. An existing locator means the source already has a link,
  //    so we never create (and never re-observe). A brand-new thread is created
  //    once; its fresh issue already carries the desired title/body/labels.
  let locator = firstNonEmptyString(thread.thread_locator);
  let providerThreadId;
  let created = false;
  let contentRefreshed = false;
  let snapshot;
  if (!locator) {
    const pushed = runProvider(buildCreateFrame(thread, frameOptions(config)), config);
    locator = threadLocatorFrom(pushed);
    providerThreadId = providerThreadIdFrom(pushed);
    created = true;
    if (!locator) {
      throw new Error(`provider create yielded no locator for ${thread.identity_key}`);
    }
    snapshot = { state: "OPEN", labels: desiredLabels, comment_markers: [] };
  } else {
    // 2. Read the live issue ONCE. Every write decision below is computed from
    //    this single snapshot, so a fully-current thread costs one read and zero
    //    writes (no per-comment re-fetch).
    snapshot = readGitHubThreadSnapshot({ adapterRef: locator, env: config.env });
    if (issueContentDrift(snapshot, thread)) {
      runProvider(buildCreateFrame(thread, frameOptions(config)), config);
      contentRefreshed = true;
    }
  }

  // 3. Reconcile labels + open/closed only when the snapshot shows drift.
  const present = new Set(snapshot.labels);
  const desiredSet = new Set(desiredLabels);
  const labelAdds = desiredLabels.filter((label) => !present.has(label));
  const labelRemoves = managedLabels.filter((label) => !desiredSet.has(label) && present.has(label));
  const stateDrift = (String(snapshot.state).toUpperCase() !== "CLOSED") !== (thread.state === "open");
  if (labelAdds.length > 0 || labelRemoves.length > 0 || stateDrift) {
    runProvider(buildLifecycleFrame(thread, locator, frameOptions(config)), config);
  }

  // 4. Post only comments whose marker is not already on the issue.
  const presentComments = new Set(snapshot.comment_markers);
  const presentCommentBodies = new Set(
    (Array.isArray(snapshot.comment_bodies) ? snapshot.comment_bodies : []).map(normalizeMarkdown),
  );
  const missingComments = comments.filter((comment) =>
    !presentComments.has(comment.entry_id)
    && !presentCommentBodies.has(normalizeMarkdown(comment.body))
  );
  for (const comment of missingComments) {
    runProvider(buildMessageFrame(thread, comment, locator, frameOptions(config)), config);
  }

  return {
    summary: {
      identity_key: thread.identity_key,
      locator,
      created,
      content_refreshed: contentRefreshed,
      state: thread.state,
      labels_changed: labelAdds.length + labelRemoves.length + (stateDrift ? 1 : 0),
      comments_posted: missingComments.length,
    },
    // Observe every reconciled thread so the source advances its per-thread
    // cursor (via the echoed watermark) and stops returning this thread until it
    // changes again. The source only sends changed threads, so this stays cheap.
    observation: buildObservation(thread, locator, providerThreadId),
  };
}

function retireOrphanManagedThreads({ config, threads, results }) {
  if (!config.fullReconcile || !config.targetRepo) return [];
  const managedLabels = uniqueStrings(threads.flatMap((thread) => thread.managed_labels ?? []));
  if (managedLabels.length === 0) return [];

  const desiredLocators = new Set();
  for (const thread of threads) {
    const locator = safeIssueRef(thread.thread_locator)?.thread_locator;
    if (locator) desiredLocators.add(locator);
  }
  for (const result of results) {
    const locator = safeIssueRef(result.locator)?.thread_locator;
    if (locator) desiredLocators.add(locator);
  }

  const providerIssues = listGitHubIssuesWithAnyLabel({
    repoSlug: config.targetRepo,
    labels: managedLabels,
    env: config.env,
  });
  const retired = [];
  for (const issue of providerIssues) {
    const issueRef = safeIssueRef(issue.thread_locator);
    if (!issueRef || desiredLocators.has(issueRef.thread_locator)) continue;
    const managedPresent = uniqueStrings((issue.labels ?? []).filter((label) => managedLabels.includes(label)));
    if (managedPresent.length === 0) continue;

    const thread = {
      schema_version: 1,
      provider: "github",
      target_repo: config.targetRepo,
      identity_key: `orphan:${config.targetRepo}#${issueRef.issue_number}`,
      thread_locator: issueRef.thread_locator,
      title: firstNonEmptyString(issue.title, `Orphaned managed thread #${issueRef.issue_number}`),
      body: "This provider thread is no longer present in the source desired-state payload.",
      labels: [],
      managed_labels: managedLabels,
      state: "closed",
      close_reason: "not_planned",
      comments: [],
    };
    const stateDrift = String(issue.state ?? "").toUpperCase() !== "CLOSED";
    if (config.dryRun) {
      retired.push({
        identity_key: thread.identity_key,
        locator: issueRef.thread_locator,
        orphaned: true,
        dry_run: true,
        labels_changed: managedPresent.length,
        state_changed: stateDrift,
      });
      continue;
    }
    runProvider(buildLifecycleFrame(thread, issueRef.thread_locator, frameOptions(config)), config);
    retired.push({
      identity_key: thread.identity_key,
      locator: issueRef.thread_locator,
      orphaned: true,
      state: "closed",
      labels_changed: managedPresent.length + (stateDrift ? 1 : 0),
    });
  }
  return retired;
}

function uniqueStrings(values) {
  return [...new Set(
    values
      .map((value) => firstNonEmptyString(value))
      .filter((value) => value !== undefined),
  )];
}

function issueContentDrift(snapshot, thread) {
  const desiredTitle = firstNonEmptyString(thread.title);
  const desiredBody = firstNonEmptyString(thread.body);
  const titleDrift = desiredTitle ? firstNonEmptyString(snapshot.title) !== desiredTitle : false;
  const bodyDrift = desiredBody ? normalizeMarkdown(snapshot.body) !== normalizeMarkdown(desiredBody) : false;
  return titleDrift || bodyDrift;
}

function normalizeMarkdown(value) {
  return String(value ?? "").replace(/\r\n/g, "\n").trim();
}

function buildObservation(thread, locator, providerThreadId) {
  if (!locator) return undefined;
  const issueRef = safeIssueRef(locator);
  return prune({
    schema_version: 1,
    provider: thread.provider,
    target_repo: thread.target_repo,
    identity_key: thread.identity_key,
    thread_locator: issueRef?.thread_locator ?? locator,
    thread_url: issueRef?.issue_url ?? locator,
    provider_thread_id: firstNonEmptyString(providerThreadId, issueRef?.issue_number),
    ref: thread.ref,
    observed_at: new Date().toISOString(),
  });
}

function frameOptions(config) {
  return { adapterId: config.adapterId, sourceId: config.sourceId };
}

function readConfig(env) {
  const apiBaseUrl = trim(env.THREAD_SYNC_API_BASE_URL);
  if (!apiBaseUrl) {
    throw new Error("THREAD_SYNC_API_BASE_URL is required.");
  }
  const internalSecret = trim(env.THREAD_SYNC_INTERNAL_SECRET);
  if (!internalSecret) {
    throw new Error("THREAD_SYNC_INTERNAL_SECRET is required.");
  }
  const fullReconcile =
    env.THREAD_SYNC_FULL_RECONCILE === "1"
    || env.THREAD_SYNC_FULL_RECONCILE === "true"
    || env.THREAD_SYNC_MODE === "full";
  return {
    apiBaseUrl: apiBaseUrl.replace(/\/+$/, ""),
    internalSecret,
    provider: trim(env.THREAD_SYNC_PROVIDER) ?? "github",
    targetRepo: trim(env.THREAD_SYNC_TARGET_REPO),
    sourceId: trim(env.THREAD_SYNC_SOURCE_ID) ?? "tenant",
    adapterId: trim(env.THREAD_SYNC_ADAPTER_ID) ?? "runx-github-thread-adapter",
    // No cap by default: reconcile every changed thread the source returns. A
    // limit is only for bounded test/repair runs.
    limit: optionalPositiveInteger(env.THREAD_SYNC_LIMIT),
    fullReconcile,
    dryRun: env.THREAD_SYNC_DRY_RUN === "1" || env.THREAD_SYNC_DRY_RUN === "true",
    continueOnError: env.THREAD_SYNC_FAIL_FAST !== "1" && env.THREAD_SYNC_FAIL_FAST !== "true",
    progressEvery: optionalPositiveInteger(env.THREAD_SYNC_PROGRESS_EVERY) ?? (fullReconcile ? 1 : 10),
    providerTimeoutMs: positiveInteger(env.THREAD_SYNC_PROVIDER_TIMEOUT_MS, 60_000),
    env,
  };
}

async function fetchDesiredState(config) {
  const url = new URL("/internal/thread-desired-state", config.apiBaseUrl);
  url.searchParams.set("provider", config.provider);
  if (config.limit) {
    url.searchParams.set("limit", String(config.limit));
  }
  if (config.targetRepo) {
    url.searchParams.set("target_repo", config.targetRepo);
  }
  if (config.fullReconcile) {
    url.searchParams.set("mode", "full");
  }
  const response = await fetch(url, {
    headers: { authorization: `Bearer ${config.internalSecret}` },
  });
  if (!response.ok) {
    throw new Error(`thread desired-state fetch failed: ${response.status} ${await response.text()}`);
  }
  return response.json();
}

async function postObservations(config, observations) {
  const response = await fetch(new URL("/internal/thread-state-observations", config.apiBaseUrl), {
    method: "POST",
    headers: {
      authorization: `Bearer ${config.internalSecret}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({ schema_version: 1, observations }),
  });
  if (!response.ok) {
    throw new Error(`thread observation post failed: ${response.status} ${await response.text()}`);
  }
}

async function flushObservations(config, observations) {
  if (config.dryRun || observations.length === 0) {
    return 0;
  }
  const batch = observations.slice();
  await postObservations(config, batch);
  observations.length = 0;
  return batch.length;
}

function runProvider(frame, config) {
  const child = spawnSync(process.execPath, [PROVIDER_SCRIPT], {
    input: JSON.stringify(frame),
    encoding: "utf8",
    env: config.env ?? process.env,
    timeout: config.providerTimeoutMs,
  });
  if (child.error) {
    throw new Error(`thread provider failed for ${frame.outbox_entry_id}: ${child.error.message}`);
  }
  if (child.status !== 0) {
    throw new Error([
      `thread provider failed for ${frame.outbox_entry_id}.`,
      child.stderr?.trim(),
      child.stdout?.trim(),
    ].filter(Boolean).join("\n"));
  }
  return JSON.parse(child.stdout);
}

function logSyncEvent(event, fields) {
  process.stderr.write(`[thread-sync] ${event} ${JSON.stringify(prune(fields) ?? {})}\n`);
}

function writeJsonLine(payload) {
  return new Promise((resolve, reject) => {
    process.stdout.write(`${JSON.stringify(payload)}\n`, (error) => {
      if (error) {
        reject(error);
        return;
      }
      resolve();
    });
  });
}

function logThreadProgress(config, fields) {
  const index = Number.isInteger(fields.index) ? fields.index : 0;
  const total = Number.isInteger(fields.total) ? fields.total : 0;
  const isLast = total > 0 && index === total - 1;
  if (!isLast && (index + 1) % config.progressEvery !== 0) {
    return;
  }
  logSyncEvent("thread", {
    position: `${index + 1}/${total}`,
    identity_key: fields.identity_key,
    locator: fields.locator,
    created: fields.created,
    labels_changed: fields.labels_changed,
    comments_posted: fields.comments_posted,
    dry_run: fields.dry_run,
    duration_ms: fields.duration_ms,
    error: fields.error,
  });
}

function threadLocatorFrom(pushed) {
  const output = pushed?.output ?? {};
  return firstNonEmptyString(
    output.outbox_entry?.thread_locator,
    output.push?.provider_thread?.thread_locator,
    output.push?.lifecycle?.locator && safeIssueRef(output.push.lifecycle.locator)?.thread_locator,
  );
}

function providerThreadIdFrom(pushed) {
  const output = pushed?.output ?? {};
  return firstNonEmptyString(
    output.outbox_entry?.metadata?.provider_thread_id,
    output.push?.provider_thread?.issue_number,
  );
}

function safeIssueRef(locator) {
  try {
    return parseGitHubIssueRef(locator);
  } catch {
    return undefined;
  }
}

function messageOf(error) {
  return error instanceof Error ? error.message : String(error);
}

function positiveInteger(value, fallback) {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function optionalPositiveInteger(value) {
  const parsed = positiveInteger(value, 0);
  return parsed > 0 ? parsed : undefined;
}

function trim(value) {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : undefined;
}

main().then(() => {
  process.exit(0);
}).catch((error) => {
  console.error(messageOf(error));
  process.exit(1);
});
