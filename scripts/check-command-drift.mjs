#!/usr/bin/env node
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const scannedRoots = ["README.md", "docs", "skills", "examples", "crates", "packages", "scripts"];
const retiredSurfaces = [
  {
    pattern: /\brunx\s+skill\s+add\b/u,
    message: "retired command shape 'runx skill add'; use 'runx add <ref>' for installs",
  },
  {
    pattern: /\brunx\s+skill\s+search\b/u,
    message: "retired command shape 'runx skill search'; use 'runx registry search <query>'",
  },
  {
    pattern: /\brunx\s+skill\s+publish\b/u,
    message: "retired command shape 'runx skill publish'; use 'runx registry publish <skill-dir|SKILL.md>'",
  },
  {
    pattern: /\brunx\s+connect\s+github\b/u,
    message: "retired connect copy 'runx connect github'; use 'runx login --provider github --for publish'",
  },
  {
    pattern: /--(?:api-url|local-api|apiBaseUrl|allowLocalApi|registryDir|trustTier|destination|prefetchOfficial)\b/u,
    message: "retired CLI flag alias; use the canonical kebab-case flag documented by native runx",
  },
];
const failures = [];

checkCommandRegistryParity(failures);

for (const relativePath of scannedRoots.flatMap((entry) => textFiles(entry))) {
  const source = readFileSync(path.join(root, relativePath), "utf8");
  for (const retired of retiredSurfaces) {
    if (retired.pattern.test(source) && !allowedRetiredCommandReference(relativePath, source)) {
      failures.push(`${relativePath}: ${retired.message}`);
    }
  }
}

if (failures.length > 0) {
  for (const failure of failures) console.error(failure);
  process.exit(1);
}

console.log("command drift check ok");

function checkCommandRegistryParity(output) {
  const registrySource = readFileSync(
    path.join(root, "crates/runx-cli/src/command_spec/catalog.rs"),
    "utf8",
  );
  const registryNames = new Set(
    [...registrySource.matchAll(/CommandSpec \{\s*name: "([a-z][a-z0-9-]*)"/gu)]
      .map((match) => match[1]),
  );
  const matrix = JSON.parse(readFileSync(path.join(root, "fixtures/cli-parity/commands.json"), "utf8"));
  const matrixNames = new Set(
    matrix.commands
      .filter((command) => command.id !== "cli.help")
      .map((command) => command.id.split(".")[0]),
  );
  for (const name of registryNames) {
    if (!matrixNames.has(name)) {
      output.push(`command registry '${name}' is missing from fixtures/cli-parity/commands.json`);
    }
  }
  for (const name of matrixNames) {
    if (!registryNames.has(name)) {
      output.push(`CLI parity command '${name}' has no crates/runx-cli command registry entry`);
    }
  }

  const registryFlags = new Map(
    [...registrySource.matchAll(/CommandSpec \{([\s\S]*?)\n    \},/gu)].map((match) => {
      const name = match[1].match(/\bname: "([a-z][a-z0-9-]*)"/u)?.[1];
      const flags = new Set(
        [...match[1].matchAll(/(?:^|[\s"[,|])(--[a-z][a-z0-9-]*|-[A-Za-z])(?=[\s",|\]=]|$)/gmu)]
          .map((flagMatch) => flagMatch[1]),
      );
      return [name, flags];
    }).filter(([name]) => name),
  );
  const parityFlags = new Map();
  for (const command of matrix.commands) {
    if (command.id === "cli.help") continue;
    const name = command.id.split(".")[0];
    const flags = parityFlags.get(name) ?? new Set();
    for (const flag of command.flags) {
      if (flag !== "--help" && flag !== "-h") flags.add(flag);
    }
    parityFlags.set(name, flags);
  }
  for (const [name, flags] of parityFlags) {
    const documented = registryFlags.get(name) ?? new Set();
    for (const flag of flags) {
      if (!documented.has(flag)) {
        output.push(`CLI parity flag '${name} ${flag}' is missing from native command help`);
      }
    }
  }
}

function textFiles(relativePath) {
  if (isIgnoredPath(relativePath)) return [];
  const absolutePath = path.join(root, relativePath);
  const stat = statSync(absolutePath);
  if (stat.isFile()) return isText(relativePath) ? [relativePath] : [];
  return readdirSync(absolutePath).flatMap((entry) => {
    const child = path.join(relativePath, entry);
    if (isIgnoredPath(child)) return [];
    const absoluteChild = path.join(root, child);
    const childStat = statSync(absoluteChild);
    if (childStat.isDirectory()) return textFiles(child);
    return isText(child) ? [child] : [];
  });
}

function isIgnoredPath(relativePath) {
  return /(?:^|\/)(?:dist|target|node_modules|\.turbo|coverage)(?:\/|$)/u.test(relativePath);
}

function isText(relativePath) {
  return /\.(?:md|mdx|json|yaml|yml|toml|ts|tsx|js|mjs|rs)$/iu.test(relativePath);
}

function allowedRetiredCommandReference(relativePath, source) {
  if (relativePath === "scripts/check-command-drift.mjs") return true;
  if (/\bremoved\b|\bretired\b|\bno longer supported\b/u.test(source)) return true;
  return /(?:^|\/)(?:test|tests|fixtures)\//u.test(relativePath);
}
