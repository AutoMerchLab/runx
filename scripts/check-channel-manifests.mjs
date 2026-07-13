import { readFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const TARGETS = {
  darwinArm64: "aarch64-apple-darwin",
  darwinX64: "x86_64-apple-darwin",
  linuxArm64: "aarch64-unknown-linux-musl",
  linuxX64: "x86_64-unknown-linux-musl",
  winX64: "x86_64-pc-windows-msvc",
};

const options = parseArgs(process.argv.slice(2));
const manifest = JSON.parse(readFileSync(options.input, "utf8"));
const failures = [];

const archiveEntriesByTarget = new Map();
for (const target of Object.values(TARGETS)) {
  const artifact = manifest.artifacts?.[target];
  if (!artifact?.file) {
    failures.push(`missing artifact metadata for ${target}`);
    continue;
  }
  const archivePath = path.join(options.archives, artifact.file);
  const entries = listArchiveEntries(archivePath);
  archiveEntriesByTarget.set(target, entries);
  const binary = target.includes("windows")
    ? `${archiveStem(manifest.version, target)}/runx.exe`
    : `${archiveStem(manifest.version, target)}/runx`;
  if (!entries.has(binary)) {
    failures.push(`${artifact.file} does not contain ${binary}`);
  }
}

const scoop = JSON.parse(readFileSync(path.join(options.channels, "scoop", "runx.json"), "utf8"));
const scoopArch = scoop.architecture?.["64bit"];
const winStem = archiveStem(manifest.version, TARGETS.winX64);
expectEqual("scoop architecture.64bit.extract_dir", scoopArch?.extract_dir, winStem);
expectEqual("scoop architecture.64bit.bin", scoopArch?.bin, "runx.exe");
expectEqual("scoop autoupdate extract_dir", scoop.autoupdate?.architecture?.["64bit"]?.extract_dir, `runx-$version-${TARGETS.winX64}`);

const wingetVersion = readFileSync(path.join(options.channels, "winget", "runxhq.runx.yaml"), "utf8");
expectIncludes("winget version manifest type", wingetVersion, "ManifestType: version");
expectIncludes("winget default locale", wingetVersion, "DefaultLocale: en-US");

const wingetLocale = readFileSync(path.join(options.channels, "winget", "runxhq.runx.locale.en-US.yaml"), "utf8");
expectIncludes("winget locale manifest type", wingetLocale, "ManifestType: defaultLocale");
expectIncludes("winget locale package name", wingetLocale, "PackageName: runx");

const wingetInstaller = readFileSync(path.join(options.channels, "winget", "runxhq.runx.installer.yaml"), "utf8");
expectIncludes("winget installer manifest type", wingetInstaller, "ManifestType: installer");
expectIncludes("winget RelativeFilePath", wingetInstaller, `RelativeFilePath: ${winStem}\\runx.exe`);
expectIncludes(
  "winget root NestedInstallerFiles",
  wingetInstaller,
  [
    "NestedInstallerFiles:",
    `  - RelativeFilePath: ${winStem}\\runx.exe`,
    "    PortableCommandAlias: runx",
  ].join("\n"),
);

const aur = readFileSync(path.join(options.channels, "aur", "PKGBUILD"), "utf8");
expectIncludes("aur x86_64 package path", aur, `runx-\${pkgver}-\${target}/runx`);
expectIncludes("aur x86_64 target branch", aur, `x86_64) target="${TARGETS.linuxX64}" ;;`);
expectIncludes("aur aarch64 target branch", aur, `aarch64) target="${TARGETS.linuxArm64}" ;;`);

const homebrew = readFileSync(path.join(options.channels, "homebrew", "runx.rb"), "utf8");
expectIncludes("homebrew nested archive install", homebrew, 'bin.install Dir["*/runx"].first => "runx"');

if (failures.length > 0) {
  console.error(JSON.stringify({ status: "failed", failures }, null, 2));
  process.exit(1);
}

console.log(JSON.stringify({ status: "ok", checked: Object.values(TARGETS).length, version: manifest.version }, null, 2));

function archiveStem(version, target) {
  return `runx-${version}-${target}`;
}

function listArchiveEntries(archivePath) {
  if (archivePath.endsWith(".zip")) {
    const result = spawnSync("python3", [
      "-c",
      "import json,sys,zipfile; print(json.dumps(zipfile.ZipFile(sys.argv[1]).namelist()))",
      archivePath,
    ], { encoding: "utf8" });
    if (result.status !== 0) {
      throw new Error(`could not list zip ${archivePath}: ${result.stderr || result.stdout}`);
    }
    return new Set(JSON.parse(result.stdout));
  }
  const result = spawnSync("tar", ["-tzf", archivePath], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`could not list tar ${archivePath}: ${result.stderr || result.stdout}`);
  }
  return new Set(result.stdout.trim().split(/\r?\n/u).filter(Boolean));
}

function expectEqual(label, actual, expected) {
  if (actual !== expected) {
    failures.push(`${label} is ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
  }
}

function expectIncludes(label, haystack, needle) {
  if (!haystack.includes(needle)) {
    failures.push(`${label} does not include ${JSON.stringify(needle)}`);
  }
}

function parseArgs(argv) {
  const parsed = { input: "", channels: "", archives: "" };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--input") {
      parsed.input = argv[++index] ?? "";
      continue;
    }
    if (arg === "--channels") {
      parsed.channels = argv[++index] ?? "";
      continue;
    }
    if (arg === "--archives") {
      parsed.archives = argv[++index] ?? "";
      continue;
    }
    throw new Error(`unknown argument: ${arg}`);
  }
  for (const [key, value] of Object.entries(parsed)) {
    if (!value) {
      throw new Error(`--${key} is required`);
    }
  }
  return parsed;
}
