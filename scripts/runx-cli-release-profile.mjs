import { spawnSync } from "node:child_process";

const repository = "runxhq/runx";
const npmPackages = [
  "@runxhq/cli",
  "@runxhq/cli-darwin-arm64",
  "@runxhq/cli-darwin-x64",
  "@runxhq/cli-linux-arm64",
  "@runxhq/cli-linux-x64",
  "@runxhq/cli-win32-x64",
];

const phase = process.argv[2];
const version = requiredEnvironment("RUNX_RELEASE_VERSION");
const channel = requiredEnvironment("RUNX_RELEASE_CHANNEL");

if (channel !== "runx-cli") fail(`unsupported release channel: ${channel}`);
if (!/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(version)) {
  fail(`invalid stable CLI version: ${version}`);
}

const tag = `cli-v${version}`;
const commit = run("git", ["rev-parse", "HEAD"]);

try {
  if (phase === "prepare") prepare();
  else if (phase === "publish") publish();
  else if (phase === "verify") await verify();
  else fail(`unknown release phase: ${phase ?? "<missing>"}`);
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}

function prepare() {
  assertCleanCheckout();
  run("git", ["fetch", "origin", "main", "--tags"]);
  assertMainCommit();
  assertRemoteTagCompatible();

  run("pnpm", ["install", "--frozen-lockfile"], { timeout: 300_000 });
  run("pnpm", ["exec", "tsx", "scripts/set-release-version.ts", "--check", version]);
  run("pnpm", ["verify:fast"], { timeout: 840_000 });
  assertCleanCheckout();

  emit({
    status: "ready",
    version,
    channel,
    commit_ref: commit,
    checks: {
      clean_checkout: true,
      head_matches_origin_main: true,
      manifests_match_version: true,
      verify_fast: true,
      remote_tag_compatible: true,
    },
  });
}

function publish() {
  assertCleanCheckout();
  run("git", ["fetch", "origin", "main", "--tags"]);
  assertMainCommit();

  const remoteCommit = remoteTagCommit();
  if (remoteCommit && remoteCommit !== commit) {
    throw new Error(`${tag} already points to ${remoteCommit}, expected ${commit}`);
  }

  if (!remoteCommit) {
    const localCommit = localTagCommit();
    if (localCommit && localCommit !== commit) {
      throw new Error(`local ${tag} points to ${localCommit}, expected ${commit}`);
    }
    if (!localCommit) run("git", ["tag", "-a", tag, "-m", `Runx CLI ${version}`]);
    run("git", ["push", "origin", `refs/tags/${tag}`]);
  }

  const publishedCommit = remoteTagCommit();
  if (publishedCommit !== commit) {
    throw new Error(`remote ${tag} did not resolve to approved commit ${commit}`);
  }

  emit({
    status: "submitted",
    version,
    channel,
    release_id: tag,
    commit_ref: commit,
    locators: [
      `https://github.com/${repository}/actions/workflows/release.yml`,
      `https://github.com/${repository}/releases/tag/${tag}`,
    ],
    checks: { remote_tag_matches_commit: true },
  });
}

async function verify() {
  const deadline = Date.now() + 840_000;
  let observation;
  while (Date.now() < deadline) {
    observation = releaseObservation();
    if (observation.ready) break;
    await new Promise((resolve) => setTimeout(resolve, 15_000));
  }

  if (!observation?.ready) {
    const missing = observation?.missing?.join(", ") || "release evidence";
    throw new Error(`timed out waiting for ${tag}: ${missing}`);
  }

  emit({
    status: "verified",
    version,
    channel,
    release_id: tag,
    commit_ref: commit,
    locators: [
      observation.releaseUrl,
      `https://www.npmjs.com/package/%40runxhq%2Fcli/v/${version}`,
    ],
    checks: {
      remote_tag_matches_commit: true,
      github_release_live: true,
      npm_selector_live: true,
      npm_native_packages_live: true,
    },
  });
}

function releaseObservation() {
  const missing = [];
  if (remoteTagCommit() !== commit) missing.push("remote tag");

  let releaseUrl = "";
  const release = tryRun("gh", [
    "release",
    "view",
    tag,
    "--repo",
    repository,
    "--json",
    "url,tagName,isDraft,isPrerelease",
  ]);
  if (release.ok) {
    const parsed = JSON.parse(release.stdout);
    if (parsed.tagName === tag && parsed.isDraft === false && parsed.isPrerelease === false) {
      releaseUrl = parsed.url;
    } else {
      missing.push("public GitHub release");
    }
  } else {
    missing.push("public GitHub release");
  }

  for (const packageName of npmPackages) {
    const result = tryRun("npm", ["view", `${packageName}@${version}`, "version", "--json"]);
    if (!result.ok || parseJsonScalar(result.stdout) !== version) missing.push(`${packageName}@${version}`);
  }

  return { ready: missing.length === 0, missing, releaseUrl };
}

function assertCleanCheckout() {
  const status = run("git", ["status", "--porcelain", "--untracked-files=all"]);
  if (status) throw new Error("release checkout must be clean");
}

function assertMainCommit() {
  const mainCommit = run("git", ["rev-parse", "origin/main"]);
  if (mainCommit !== commit) {
    throw new Error(`release HEAD ${commit} does not match origin/main ${mainCommit}`);
  }
}

function assertRemoteTagCompatible() {
  const remoteCommit = remoteTagCommit();
  if (remoteCommit && remoteCommit !== commit) {
    throw new Error(`${tag} already points to ${remoteCommit}, expected ${commit}`);
  }
}

function localTagCommit() {
  const result = tryRun("git", ["rev-list", "-n", "1", tag]);
  return result.ok ? result.stdout : "";
}

function remoteTagCommit() {
  const result = tryRun("git", [
    "ls-remote",
    "origin",
    `refs/tags/${tag}`,
    `refs/tags/${tag}^{}`,
  ]);
  if (!result.ok || !result.stdout) return "";
  const refs = new Map(result.stdout.split("\n").map((line) => {
    const [objectId, ref] = line.trim().split(/\s+/, 2);
    return [ref, objectId];
  }));
  return refs.get(`refs/tags/${tag}^{}`) || refs.get(`refs/tags/${tag}`) || "";
}

function parseJsonScalar(value) {
  try {
    return JSON.parse(value);
  } catch {
    return "";
  }
}

function requiredEnvironment(name) {
  const value = process.env[name]?.trim();
  if (!value) fail(`${name} is required`);
  return value;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: process.cwd(),
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    timeout: options.timeout ?? 120_000,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const detail = [result.stdout, result.stderr].map((value) => value?.trim()).filter(Boolean).join("\n");
    throw new Error(`${command} ${args.join(" ")} failed${detail ? `: ${detail}` : ""}`);
  }
  return result.stdout.trim();
}

function tryRun(command, args) {
  try {
    return { ok: true, stdout: run(command, args) };
  } catch (error) {
    return { ok: false, stdout: "", error };
  }
}

function emit(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

function fail(message) {
  process.stderr.write(`${message}\n`);
  process.exit(1);
}
