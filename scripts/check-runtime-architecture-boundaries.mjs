#!/usr/bin/env node
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const workspaceRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const currentScriptPath = fileURLToPath(import.meta.url);
const phase = readOption("--phase");
const findings = [];

checkNormativeArchitectureContract();
checkCrateDependencyDirection();
checkCloudOwnershipBoundary();
checkManagedAgentDefault();
checkTypedCapabilityPlane();
checkDataOperationOwnership();
checkNoRuntimeCompatModules();
checkCanonicalParserOwnership();
checkCliCommandOwnership();
checkRegistryOwnership();
checkHttpTransportOwnership();
checkExternalAdapterOwnership();
checkAuthoringOwnership();
checkContractBindingOwnership();
checkGeneratedMirrorOwnership();
checkCanonicalToolManifestOwnership();
checkRetiredRuntimeSurfaces();

if (phase === "services") {
  checkServiceBoundary();
} else if (phase === "execution-split") {
  checkExecutionSplit();
} else if (phase === "projection-hot-paths") {
  checkProjectionHotPaths();
} else if (phase === "session-pooling") {
  checkSessionPooling();
} else if (phase !== undefined) {
  findings.push(`unknown runtime architecture phase '${phase}'`);
}

if (findings.length > 0) {
  console.error("Runtime architecture boundary check failed:");
  for (const finding of findings) {
    console.error(`- ${finding}`);
  }
  process.exit(1);
}

console.log(phase ? `Runtime architecture boundary check passed for ${phase}.` : "Runtime architecture boundary check passed.");

function readOption(name) {
  const index = process.argv.indexOf(name);
  if (index < 0) {
    return undefined;
  }
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) {
    findings.push(`${name} requires a value`);
    return undefined;
  }
  return value;
}

function checkNormativeArchitectureContract() {
  const architecturePath = path.join(workspaceRoot, "docs/architecture/runx-system.md");
  if (!existsSync(architecturePath)) {
    findings.push("docs/architecture/runx-system.md is missing");
    return;
  }
  const source = readFileSync(architecturePath, "utf8");
  for (const heading of [
    "## Repository ownership",
    "## Skill knowledge contract",
    "## Execution lanes",
    "## Deterministic module boundary",
    "## Native capability boundary",
    "## Effect and finality boundary",
    "## Authoring and extension boundary",
    "## Cloud boundary",
    "## Performance contract",
    "## Replacement rule",
  ]) {
    if (!source.includes(heading)) {
      findings.push(`docs/architecture/runx-system.md lacks normative section ${heading}`);
    }
  }
  for (const invariant of [
    "Graph/package authors do not supply `effect_family`",
    "It yields `needs_agent` by default",
    "There is no Node or shell fallback",
    "caller input can only narrow that set",
    "Never extend a Cloud dogfood script as a substitute",
  ]) {
    if (!source.includes(invariant)) {
      findings.push(`docs/architecture/runx-system.md lacks invariant ${JSON.stringify(invariant)}`);
    }
  }
}

function checkCrateDependencyDirection() {
  const forbidden = new Map([
    ["runx-contracts", ["runx-core", "runx-parser", "runx-receipts", "runx-runtime", "runx-cli"]],
    ["runx-core", ["runx-parser", "runx-receipts", "runx-runtime", "runx-cli"]],
    ["runx-parser", ["runx-receipts", "runx-runtime", "runx-cli"]],
    ["runx-receipts", ["runx-core", "runx-parser", "runx-runtime", "runx-cli"]],
    ["runx-runtime", ["runx-cli"]],
  ]);
  for (const [crateName, dependencies] of forbidden) {
    const manifestPath = path.join(workspaceRoot, "crates", crateName, "Cargo.toml");
    if (!existsSync(manifestPath)) {
      findings.push(`missing crate manifest ${relative(manifestPath)}`);
      continue;
    }
    const source = readFileSync(manifestPath, "utf8");
    for (const dependency of dependencies) {
      if (new RegExp(`^${escapeRegExp(dependency)}\\s*=`, "mu").test(source)) {
        findings.push(`${relative(manifestPath)} violates dependency direction with ${dependency}`);
      }
    }
  }
}

function checkDataOperationOwnership() {
  for (const relPath of [
    "skills/data-store/tools/data/local",
    "skills/data-store/tools/data/sqlite",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} is superseded by the native event-store implementation`);
    }
  }

  const nativePath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/tool_catalogs/native/event_store.rs",
  );
  const nativeSource = existsSync(nativePath) ? readFileSync(nativePath, "utf8") : "";
  for (const toolRef of [
    "data.append_event",
    "data.read_events",
    "data.read_projection",
    "data.list_stream_heads",
  ]) {
    if (!nativeSource.includes(`\"${toolRef}\"`)) {
      findings.push(`${relative(nativePath)} must own exact native operation ${toolRef}`);
    }
  }
  const inputPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/tool_catalogs/native/event_store/input.rs",
  );
  const inputSource = existsSync(inputPath) ? readFileSync(inputPath, "utf8") : "";
  if (/data_source_binding/u.test(inputSource)) {
    findings.push(`${relative(inputPath)} exposes runtime-owned data binding as a public capability input`);
  }
  const dispatchPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/tool_catalogs/dispatch.rs",
  );
  const dispatchSource = existsSync(dispatchPath) ? readFileSync(dispatchPath, "utf8") : "";
  for (const token of ["prepare_data_operation", "validate_result", "InvocationContract::DataAdapter"]) {
    if (!dispatchSource.includes(token)) {
      findings.push(`${relative(dispatchPath)} must enforce the native data contract through ${token}`);
    }
  }
  const redisManifestPath = path.join(
    workspaceRoot,
    "skills/data-store/tools/data/redis/manifest.json",
  );
  if (existsSync(redisManifestPath)) {
    const redisManifest = JSON.parse(readFileSync(redisManifestPath, "utf8"));
    const inputNames = Object.keys(redisManifest.inputs ?? {}).sort();
    if (JSON.stringify(inputNames) !== JSON.stringify(["data_source_binding", "operation"])) {
      findings.push(`${relative(redisManifestPath)} must declare only runtime-owned adapter routing inputs`);
    }
  }

  const forbiddenTokens = [
    ["data.source", /\bdata\.source\b/u],
    ["data.local", /\bdata\.local\b/u],
    ["store_id", /\bstore_id\b/u],
  ];
  for (const root of ["skills", "docs", "tests", "crates/runx-runtime/src"]) {
    const absoluteRoot = path.join(workspaceRoot, root);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (!/\.(?:md|ya?ml|json|rs|js|mjs|ts)$/u.test(filePath)) continue;
      if (filePath === currentScriptPath) continue;
      const source = readFileSync(filePath, "utf8");
      for (const [token, pattern] of forbiddenTokens) {
        if (pattern.test(source)) {
          findings.push(`${relative(filePath)} retains retired data-operation surface ${token}`);
        }
      }
    }
  }

  for (const relPath of [
    "crates/runx-runtime/src/tool_catalogs/native/event_store",
    "skills/data-store/tools/data/redis",
  ]) {
    const absoluteRoot = path.join(workspaceRoot, relPath);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (!/\.(?:rs|mjs)$/u.test(filePath)) continue;
      const source = readFileSync(filePath, "utf8");
      if (/\bevent_digests\s*:/u.test(source) || /["']event_digests["']\.to_owned\(\)\s*,/u.test(source)) {
        findings.push(`${relative(filePath)} retains an unbounded full-history projection`);
      }
    }
  }
}

function checkCanonicalToolManifestOwnership() {
  const manifestPaths = [];
  const toolRoot = path.join(workspaceRoot, "tools");
  for (const filePath of existsSync(toolRoot) ? walk(toolRoot) : []) {
    if (path.basename(filePath) === "manifest.json") manifestPaths.push(filePath);
  }
  const skillRoot = path.join(workspaceRoot, "skills");
  for (const filePath of existsSync(skillRoot) ? walk(skillRoot) : []) {
    const parts = path.relative(skillRoot, filePath).split(path.sep);
    if (path.basename(filePath) === "manifest.json" && parts.includes("tools")) {
      manifestPaths.push(filePath);
    }
  }

  for (const manifestPath of manifestPaths) {
    let manifest;
    try {
      manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    } catch {
      findings.push(`${relative(manifestPath)} is not valid JSON`);
      continue;
    }
    if (manifest.schema !== "runx.tool.manifest.v1") {
      findings.push(`${relative(manifestPath)} must declare schema runx.tool.manifest.v1`);
    }
    for (const field of [
      "output",
      "runx",
      "runtime",
      "schema_hash",
      "source_hash",
      "toolkit_version",
    ]) {
      if (Object.hasOwn(manifest, field)) {
        findings.push(`${relative(manifestPath)} duplicates derived runtime ownership through ${field}`);
      }
    }
  }
}

function checkCloudOwnershipBoundary() {
  const roots = ["crates", "packages", "skills", "src"];
  const extensions = new Set([".rs", ".ts", ".tsx", ".js", ".mjs", ".cjs"]);
  const cloudReference = /(?:\.\.\/)+cloud(?:\/|\b)|\brunx\/cloud\b|\/cloud\//u;
  for (const root of roots) {
    const absoluteRoot = path.join(workspaceRoot, root);
    if (!existsSync(absoluteRoot)) {
      continue;
    }
    for (const filePath of walk(absoluteRoot)) {
      if (!extensions.has(path.extname(filePath))) {
        continue;
      }
      if (cloudReference.test(readFileSync(filePath, "utf8"))) {
        findings.push(`${relative(filePath)} reaches into the Cloud tree from an OSS production surface`);
      }
    }
  }
}

function checkManagedAgentDefault() {
  const orchestratorPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/execution/orchestrator.rs",
  );
  const source = existsSync(orchestratorPath) ? readFileSync(orchestratorPath, "utf8") : "";
  if (!/#\[derive\([^\]]*Default[^\]]*\)\][\s\S]{0,300}enum\s+ManagedAgentPolicy[\s\S]{0,180}#\[default\]\s*HostDriven/u.test(source)) {
    findings.push(`${relative(orchestratorPath)} must default managed-agent execution to HostDriven`);
  }
  const cliParserPath = path.join(workspaceRoot, "crates/runx-cli/src/skill/parser.rs");
  const cliSource = existsSync(cliParserPath) ? readFileSync(cliParserPath, "utf8") : "";
  const managedAgentPath = path.join(workspaceRoot, "crates/runx-cli/src/managed_agent.rs");
  const managedAgentSource = existsSync(managedAgentPath)
    ? readFileSync(managedAgentPath, "utf8")
    : "";
  const skillUsesSharedPolicy = /managed_agent_policy\(\s*"skill"/u.test(cliSource);
  const sharedPolicyRequiresConsent = /if\s+!enabled\s*\{[\s\S]{0,180}max_rounds\.is_some\(\)[\s\S]{0,220}--managed-agent-rounds requires --managed-agent/u.test(
    managedAgentSource,
  );
  if (!skillUsesSharedPolicy || !sharedPolicyRequiresConsent) {
    findings.push("managed-agent policy must reject a round budget without explicit consent through the shared CLI policy");
  }
}

function checkTypedCapabilityPlane() {
  const legacyCatalog = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/tool_catalogs/native.rs",
  );
  if (existsSync(legacyCatalog)) {
    findings.push(`${relative(legacyCatalog)} must be replaced by module-owned capabilities`);
  }

  const roots = [
    "crates/runx-runtime/src/tool_catalogs/native",
    "crates/runx-runtime/src/effects",
    "crates/runx-pay/src/planning",
  ];
  const forbidden = [
    /\bstruct\s+NativeInput\b/u,
    /\bstruct\s+NativeTool\b/u,
    /\bEffectToolContract\b/u,
    /\bEffectToolInput\b/u,
  ];
  for (const root of roots) {
    for (const filePath of rustFiles(root)) {
      const source = readFileSync(filePath, "utf8");
      for (const pattern of forbidden) {
        if (pattern.test(source)) {
          findings.push(`${relative(filePath)} retains parallel capability metadata ${pattern}`);
        }
      }
    }
  }

  const capabilityPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/capability.rs",
  );
  const capability = existsSync(capabilityPath)
    ? readFileSync(capabilityPath, "utf8")
    : "";
  for (const token of [
    "trait CapabilityContract",
    "fn input_schema",
    "fn output_schema",
    "fn normalize_inputs",
    "fn validate_output",
  ]) {
    if (!capability.includes(token)) {
      findings.push(`${relative(capabilityPath)} lacks typed capability contract token ${token}`);
    }
  }
}

function checkNoRuntimeCompatModules() {
  for (const filePath of rustFiles("crates/runx-runtime/src")) {
    const source = readFileSync(filePath, "utf8");
    const rel = relative(filePath);
    if (/\bmod\s+\w+_(?:legacy|compat)\b/u.test(source)) {
      findings.push(`${rel} declares a legacy/compat runtime module`);
    }
    if (/\b(?:LegacyExecutor|CompatExecutor)\b/u.test(source)) {
      findings.push(`${rel} declares legacy executor compatibility vocabulary`);
    }
  }
}

function checkCanonicalParserOwnership() {
  const cliManifestPath = path.join(workspaceRoot, "crates/runx-cli/Cargo.toml");
  const cliManifest = existsSync(cliManifestPath) ? readFileSync(cliManifestPath, "utf8") : "";
  if (/^serde_(?:norway|yaml|yml)\s*=/mu.test(cliManifest)) {
    findings.push(`${relative(cliManifestPath)} depends on a YAML backend instead of runx-parser`);
  }

  for (const relPath of [
    "tools/spec/normalize_scafld_frontmatter",
    "tools/spec/read_declared_files",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} parses scafld-owned Markdown outside scafld`);
    }
  }
  for (const relPath of [
    "tests/http-cached-registry-store.test.ts",
    "tests/registry-fixtures.ts",
    "tests/util-split-skill-id.test.ts",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} retains a parallel TypeScript registry implementation`);
    }
  }

  const parserRoot = path.join(workspaceRoot, "crates/runx-parser");
  for (const filePath of rustFiles("crates")) {
    if (filePath.startsWith(`${parserRoot}${path.sep}`) || filePath.includes(`${path.sep}tests${path.sep}`)) {
      continue;
    }
    const source = readFileSync(filePath, "utf8");
    if (/\b(?:serde_norway|serde_yaml|serde_yml|yaml_rust)::/u.test(source)) {
      findings.push(`${relative(filePath)} parses YAML outside the canonical runx-parser crate`);
    }
  }

  const runtimeFacade = path.join(workspaceRoot, "crates/runx-runtime/src/parser_eval.rs");
  if (existsSync(runtimeFacade)) {
    findings.push(`${relative(runtimeFacade)} is a redundant parser facade; callers must depend on runx-parser`);
  }
  const runtimeHarnessFixturePath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/execution/harness/fixtures.rs",
  );
  const runtimeHarnessFixture = existsSync(runtimeHarnessFixturePath)
    ? productionRustSource(readFileSync(runtimeHarnessFixturePath, "utf8"))
    : "";
  if (/pub\s+fn\s+parse_harness_fixture|\bproject_parser_error\b|HarnessFixtureError::(?:Required|Empty|Invalid|RetiredReceiptField|UnknownReceiptField|UnsupportedFixtureMode)/u.test(runtimeHarnessFixture)) {
    findings.push(`${relative(runtimeHarnessFixturePath)} mirrors parser-owned harness parsing or diagnostics`);
  }
  const runtimeDevLoopPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/dev/loop.rs",
  );
  const runtimeDevLoop = existsSync(runtimeDevLoopPath)
    ? productionRustSource(readFileSync(runtimeDevLoopPath, "utf8"))
    : "";
  if (/\bparse_yaml_document\b|\bParsedDevFixture\b|fixture\.document|json_(?:object|string)_field\([^\n]*fixture/u.test(runtimeDevLoop)) {
    findings.push(`${relative(runtimeDevLoopPath)} reparses parser-owned dev fixture contracts`);
  }
  const parserLibPath = path.join(workspaceRoot, "crates/runx-parser/src/lib.rs");
  const parserLib = existsSync(parserLibPath) ? readFileSync(parserLibPath, "utf8") : "";
  if (!parserLib.includes("parse_dev_fixture") || !parserLib.includes("DevFixture")) {
    findings.push(`${relative(parserLibPath)} must own the typed runx dev fixture contract`);
  }
  const runtimeConfigPath = path.join(workspaceRoot, "crates/runx-runtime/src/config.rs");
  const runtimeConfig = existsSync(runtimeConfigPath)
    ? productionRustSource(readFileSync(runtimeConfigPath, "utf8"))
    : "";
  if (/parse_yaml_document[\s\S]{0,240}manifest|manifest_text[\s\S]{0,240}JsonValue::Object/u.test(runtimeConfig)) {
    findings.push(`${relative(runtimeConfigPath)} reparses runner profile manifests outside runx-parser`);
  }
  const runtimeLib = path.join(workspaceRoot, "crates/runx-runtime/src/lib.rs");
  const runtimeSource = existsSync(runtimeLib) ? readFileSync(runtimeLib, "utf8") : "";
  if (/\b(?:ParserEvalError|ParserEvalOutput|evaluate_parser_document_str|parse_yaml_document)\b/u.test(runtimeSource)) {
    findings.push(`${relative(runtimeLib)} re-exports parser ownership through the runtime`);
  }

  for (const root of ["scripts", "tests"]) {
    const absoluteRoot = path.join(workspaceRoot, root);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (!/\.(?:js|mjs|cjs|ts)$/u.test(filePath)) continue;
      const source = readFileSync(filePath, "utf8");
      if (/from\s+["']yaml["']|require\(["']yaml["']\)/u.test(source)) {
        findings.push(`${relative(filePath)} parses YAML outside the canonical native parser`);
      }
      if (
        filePath !== path.join(workspaceRoot, "scripts/check-runtime-architecture-boundaries.mjs")
        && /\b(?:assertExecutionProfileYamlSubset|parseSkillFrontmatter)\b/u.test(source)
      ) {
        findings.push(`${relative(filePath)} reimplements a canonical parser contract`);
      }
    }
  }
  const readinessPath = path.join(workspaceRoot, "scripts/check-readiness-structural.mjs");
  const readinessSource = existsSync(readinessPath) ? readFileSync(readinessPath, "utf8") : "";
  if (/\bextractFrontmatterField\b/u.test(readinessSource)) {
    findings.push(`${relative(readinessPath)} must use scafld-owned path identity, not parse lifecycle front matter`);
  }
  for (const filePath of skillProductionFiles()) {
    const source = readFileSync(filePath, "utf8");
    if (/\bparse(?:Skill)?Frontmatter\b|\bparse_frontmatter\b/u.test(source)) {
      findings.push(`${relative(filePath)} reimplements package frontmatter parsing outside runx-parser`);
    }
  }
  const packageManifest = JSON.parse(readFileSync(path.join(workspaceRoot, "package.json"), "utf8"));
  if (packageManifest.dependencies?.yaml || packageManifest.devDependencies?.yaml) {
    findings.push("package.json retains the parallel JavaScript YAML parser dependency");
  }
  const parserBridgePath = path.join(workspaceRoot, "scripts/lib/native-parser.mjs");
  const parserBridge = existsSync(parserBridgePath) ? readFileSync(parserBridgePath, "utf8") : "";
  for (const token of [
    "parser\", \"eval",
    "validateRunnerManifestYamlBatch",
    "validateHarnessFixtureYamlBatch",
    "parsePacketSchemaDocumentsBatch",
  ]) {
    if (!parserBridge.includes(token)) {
      findings.push(`${relative(parserBridgePath)} lacks canonical native-parser bridge token ${token}`);
    }
  }

  const packetGeneratorPath = path.join(workspaceRoot, "scripts/generate-packet-schemas.ts");
  const packetGenerator = existsSync(packetGeneratorPath)
    ? readFileSync(packetGeneratorPath, "utf8")
    : "";
  for (const token of [
    "ownedPacketContracts",
    'schema["x-runx-schema"]',
    'path.join(workspaceRoot, "schemas")',
    "parsePacketSchemaDocumentsBatch",
    "collectManifestContracts",
  ]) {
    if (!packetGenerator.includes(token)) {
      findings.push(`${relative(packetGeneratorPath)} must discover Rust-owned packet identities from root schemas`);
      break;
    }
  }
  if (/\bcanonicalPacketContracts\b/u.test(packetGenerator)) {
    findings.push(`${relative(packetGeneratorPath)} retains a parallel packet-contract registry`);
  }
  if (/\.raw\.document|\bcollectContracts\s*\(/u.test(packetGenerator)) {
    findings.push(`${relative(packetGeneratorPath)} reparses raw runner manifests instead of using typed parser output`);
  }

  for (const filePath of walk(path.join(workspaceRoot, "scripts"))) {
    if (!/\.(?:js|mjs|cjs|ts)$/u.test(filePath) || filePath === currentScriptPath) continue;
    if (/\.raw\.document/u.test(readFileSync(filePath, "utf8"))) {
      findings.push(`${relative(filePath)} interprets raw parser output instead of typed parser IR`);
    }
  }
  const versionDriftPath = path.join(workspaceRoot, "scripts/check-skill-version-drift.mjs");
  const versionDrift = existsSync(versionDriftPath) ? readFileSync(versionDriftPath, "utf8") : "";
  if (/\b(?:consumedScripts|visitValues|normalizeScriptPath)\b/u.test(versionDrift)) {
    findings.push(`${relative(versionDriftPath)} guesses package dependencies from arbitrary manifest values`);
  }
  const skillFixtureGeneratorPath = path.join(
    workspaceRoot,
    "scripts/generate-rust-skill-fixtures.ts",
  );
  const skillFixtureGenerator = existsSync(skillFixtureGeneratorPath)
    ? readFileSync(skillFixtureGeneratorPath, "utf8")
    : "";
  if (!skillFixtureGenerator.includes("source.graph?.steps")) {
    findings.push(`${relative(skillFixtureGeneratorPath)} must consume parser-owned typed graph structure`);
  }

  const packetParserPath = path.join(workspaceRoot, "crates/runx-parser/src/packet.rs");
  const packetParser = existsSync(packetParserPath) ? readFileSync(packetParserPath, "utf8") : "";
  for (const token of ["PACKET_ID_FIELD", "parse_packet_schema_document", "ValidatedPacketSchema"]) {
    if (!packetParser.includes(token)) {
      findings.push(`${relative(packetParserPath)} lacks canonical packet parser token ${token}`);
    }
  }
  const packetCatalogPath = path.join(workspaceRoot, "crates/runx-runtime/src/packet_schemas.rs");
  const packetCatalog = existsSync(packetCatalogPath) ? readFileSync(packetCatalogPath, "utf8") : "";
  for (const token of [
    "PacketSchemaCatalog",
    "parse_packet_schema_document",
    "packet_schema_directories",
    "discover_loaded_package",
  ]) {
    if (!packetCatalog.includes(token)) {
      findings.push(`${relative(packetCatalogPath)} lacks canonical packet catalog token ${token}`);
    }
  }
  const parallelPacketConsumers = [
    ["crates/runx-runtime/src/list.rs", /\bstruct\s+PacketSchema\b|\bfn\s+packet_id\s*\(/u],
    ["crates/runx-runtime/src/packet_validation.rs", /\bdiscover_packet_schemas\b|\bstruct\s+PacketSchema\b/u],
    ["crates/runx-runtime/src/registry/publish_package/files/packet.rs", /\bdiscover_packet_schemas\b|\bread_packet_schema\b|\bserde_json\b|\bstd::fs\b/u],
  ];
  for (const [relPath, pattern] of parallelPacketConsumers) {
    const filePath = path.join(workspaceRoot, relPath);
    const source = existsSync(filePath) ? productionRustSource(readFileSync(filePath, "utf8")) : "";
    if (pattern.test(source)) {
      findings.push(`${relPath} retains parallel packet parser or catalog ownership`);
    }
  }

  const graphTypesPath = path.join(workspaceRoot, "crates/runx-parser/src/graph/types.rs");
  const graphTypes = existsSync(graphTypesPath) ? readFileSync(graphTypesPath, "utf8") : "";
  if (!/pub\s+artifacts:\s+Option<SkillArtifactContract>/u.test(graphTypes)) {
    findings.push(`${relative(graphTypesPath)} must expose parser-validated graph artifact contracts`);
  }
  if (!/pub\s+run:\s+Option<GraphRunTarget>/u.test(graphTypes)) {
    findings.push(`${relative(graphTypesPath)} must expose parser-validated inline graph targets`);
  }
  const typedArtifactConsumers = [
    ["crates/runx-runtime/src/packet_validation.rs", /\binline_artifacts\b|artifacts\.get\s*\(/u],
    ["crates/runx-runtime/src/output_contract.rs", /\binline_artifacts\b/u],
    ["crates/runx-runtime/src/list.rs", /\bjson_artifact_emits\b/u],
    ["crates/runx-cli/src/registry/package.rs", /\bcollect_declared_packet_ids\b/u],
    ["crates/runx-runtime/src/adapters/agent.rs", /source\.raw[\s\S]{0,160}?artifacts/u],
  ];
  for (const [relPath, pattern] of typedArtifactConsumers) {
    const filePath = path.join(workspaceRoot, relPath);
    const source = existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
    if (pattern.test(source)) {
      findings.push(`${relPath} reparses artifact contracts outside runx-parser`);
    }
  }
  for (const filePath of rustFiles("crates")) {
    if (filePath.startsWith(`${parserRoot}${path.sep}`)) continue;
    const source = readFileSync(filePath, "utf8");
    if (/runner\.raw\.get\(\s*"scopes"\s*\)|\bcollect_declared_scopes\b/u.test(source)) {
      findings.push(`${relative(filePath)} reparses runner scopes instead of using parser-owned declared_scopes`);
    }
    if (/source\.raw\.get\(\s*"allowed_tools"\s*\)/u.test(source)) {
      findings.push(`${relative(filePath)} reparses allowed_tools instead of using the typed invocation contract`);
    }
    if (/validate_skill_source\(\s*run\b|\brun\.get\(\s*"(?:type|outputs|sandbox)"/u.test(source)) {
      findings.push(`${relative(filePath)} reparses an inline graph target instead of using GraphRunTarget`);
    }
  }
  const operatorContextPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/execution/operator_context.rs",
  );
  const operatorContext = existsSync(operatorContextPath)
    ? productionRustSource(readFileSync(operatorContextPath, "utf8"))
    : "";
  if (!/struct\s+SkillOperatorContextStep[\s\S]*?pub\s+definition:\s+GraphStep/u.test(operatorContext)) {
    findings.push(`${relative(operatorContextPath)} must retain GraphStep as the typed graph-step owner`);
  }
  if (/SkillOperatorContextTarget|struct\s+SkillOperatorContextStep[\s\S]*?pub\s+raw:\s+JsonValue/u.test(operatorContext)) {
    findings.push(`${relative(operatorContextPath)} retains a parallel graph-step target or raw contract`);
  }
  const preparedSkillPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/execution/prepared_skill.rs",
  );
  const preparedSkill = existsSync(preparedSkillPath)
    ? productionRustSource(readFileSync(preparedSkillPath, "utf8"))
    : "";
  if (/step\.raw|json_field\s*\(\s*&step\.|collect_string_values\s*\(\s*&step\./u.test(preparedSkill)) {
    findings.push(`${relative(preparedSkillPath)} reparses serialized graph steps instead of using GraphStep`);
  }
  const externalAdapterRuntimePath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/adapters/external_adapter.rs",
  );
  const externalAdapterRuntime = existsSync(externalAdapterRuntimePath)
    ? readFileSync(externalAdapterRuntimePath, "utf8")
    : "";
  if (/source\.raw|\b(?:inline_manifest_value|manifest_path_value|optional_source_string)\b/u.test(externalAdapterRuntime)) {
    findings.push(`${relative(externalAdapterRuntimePath)} reparses external-adapter source metadata outside runx-parser`);
  }
  const registryPackagePath = path.join(workspaceRoot, "crates/runx-cli/src/registry/package.rs");
  const registryPackage = existsSync(registryPackagePath) ? readFileSync(registryPackagePath, "utf8") : "";
  if (/\bcollect_keyed_string_values\b|\bcollect_script_string_values\b/u.test(registryPackage)) {
    findings.push(`${relative(registryPackagePath)} recursively guesses external-adapter sidecars instead of using typed contracts`);
  }
  const sourceTypesPath = path.join(workspaceRoot, "crates/runx-parser/src/skill/types.rs");
  const sourceTypes = existsSync(sourceTypesPath) ? readFileSync(sourceTypesPath, "utf8") : "";
  const sourceKind = sourceTypes.match(/pub enum SourceKind\s*\{([\s\S]*?)\n\}/u)?.[1] ?? "";
  for (const retired of ["Catalog", "HarnessHook", "Http"]) {
    if (new RegExp(`\\b${retired}\\b`, "u").test(sourceKind)) {
      findings.push(`${relative(sourceTypesPath)} retains retired ${retired} source ownership`);
    }
  }
  const skillSource = sourceTypes.match(/pub struct SkillSource\s*\{([\s\S]*?)\n\}/u)?.[1] ?? "";
  if (/\bpub\s+catalog_ref\s*:/u.test(skillSource)) {
    findings.push(`${relative(sourceTypesPath)} retains catalog_ref on SkillSource; graph tool steps own catalog dispatch`);
  }
  const sourceParserPath = path.join(workspaceRoot, "crates/runx-parser/src/skill/source.rs");
  const sourceParser = existsSync(sourceParserPath) ? readFileSync(sourceParserPath, "utf8") : "";
  for (const retired of ["http", "catalog"]) {
    if (!sourceParser.includes(`{field} ${retired} was removed`)) {
      findings.push(`${relative(sourceParserPath)} must fail explicitly for retired source.type ${retired}`);
    }
  }
  for (const field of ["external_adapter", "thread_outbox_provider"]) {
    if (!new RegExp(`pub\\s+${field}:\\s+Option<`).test(sourceTypes)) {
      findings.push(`${relative(sourceTypesPath)} must expose parser-owned typed ${field} metadata`);
    }
  }
  const threadOutboxPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/adapters/thread_outbox_provider.rs",
  );
  const threadOutboxSource = existsSync(threadOutboxPath)
    ? readFileSync(threadOutboxPath, "utf8")
    : "";
  if (/source\.raw|\bparse_(?:source|config)\b/u.test(threadOutboxSource)) {
    findings.push(`${relative(threadOutboxPath)} reparses thread-outbox source metadata outside runx-parser`);
  }

  for (const relPath of [
    "crates/runx-runtime/src/tool_catalogs/build.rs",
    "crates/runx-runtime/src/tool_catalogs/inspect.rs",
    "crates/runx-runtime/src/dev/tool.rs",
  ]) {
    const filePath = path.join(workspaceRoot, relPath);
    const source = existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
    if (/\bRawToolManifest\b|\bnormalize_tool_(?:manifest_shape|output)\b|\bruntime_from_source\b|artifacts\.get\s*\(/u.test(source)) {
      findings.push(`${relPath} reparses tool manifests instead of projecting parser-owned typed IR`);
    }
  }
  for (const filePath of rustFiles("crates/runx-cli/src")) {
    const source = readFileSync(filePath, "utf8");
    if (/\bcollect_(?:external_adapter|process)_script_files\b|\bprocess_script_files\b/u.test(source)) {
      findings.push(`${relative(filePath)} guesses execution sidecars instead of using parser-owned execution_files`);
    }
  }
  for (const [relPath, retired] of [
    ["crates/runx-runtime/src/execution/runner/steps.rs", /\bStepTypeRegistry\b|\bregistered_step_type\b|\brun_type_ref\b/u],
    ["crates/runx-runtime/src/execution/skill_front/graph.rs", /\bSourceAdapterRegistry\b|\bbuiltin_source_handlers\b/u],
  ]) {
    const filePath = path.join(workspaceRoot, relPath);
    const source = existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
    if (retired.test(source)) {
      findings.push(`${relPath} retains a string registry beside typed source/step dispatch`);
    }
  }

  const retiredBinaryAliases = [
    `RUNX_${"KERNEL_EVAL_BIN"}`,
    `RUNX_${"PARSER_EVAL_BIN"}`,
    `RUNX_${"DEV_RUST_CLI_BIN"}`,
  ];
  for (const root of ["scripts", "tests"]) {
    const absoluteRoot = path.join(workspaceRoot, root);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (!/\.(?:js|mjs|cjs|ts)$/u.test(filePath) || filePath === currentScriptPath) continue;
      const source = readFileSync(filePath, "utf8");
      const alias = retiredBinaryAliases.find((candidate) => source.includes(candidate));
      if (alias) {
        findings.push(`${relative(filePath)} retains binary alias ${alias}; use RUNX_RUST_CLI_BIN`);
      }
    }
  }
}

function checkCliCommandOwnership() {
  const commandSpecPath = path.join(workspaceRoot, "crates/runx-cli/src/command_spec.rs");
  const commandSpec = existsSync(commandSpecPath) ? readFileSync(commandSpecPath, "utf8") : "";
  if (!/pub fn catalog_json\(\)/u.test(commandSpec) || !/CommandCatalog/u.test(commandSpec)) {
    findings.push(`${relative(commandSpecPath)} must project the native help catalog as JSON`);
  }

  const catalogPath = path.join(workspaceRoot, "crates/runx-cli/src/command_spec/catalog.rs");
  const catalog = existsSync(catalogPath) ? readFileSync(catalogPath, "utf8") : "";
  if (!/ROOT_COMMAND_SPEC/u.test(catalog) || !/--audience https:\/\/host/u.test(catalog)) {
    findings.push(`${relative(catalogPath)} must own root help and the complete native option catalog`);
  }

  const parityPath = path.join(workspaceRoot, "scripts/generate-cli-feature-parity.ts");
  const parity = existsSync(parityPath) ? readFileSync(parityPath, "utf8") : "";
  for (const token of [
    "command(\"",
    "requiredPositionals",
    "conditionalPositionals",
    "checkHelpCoverage",
    "checkUsageCoverage",
    "command_spec/catalog.rs",
  ]) {
    if (parity.includes(token)) {
      findings.push(`${relative(parityPath)} duplicates native CLI syntax through '${token}'`);
    }
  }
  for (const required of [
    'spawnSync(runx, ["--help", "--json"]',
    'sourceOfTruth: "runx --help --json"',
    "bindNativeCommands(readNativeCommandCatalog(), commandAnnotations)",
  ]) {
    if (!parity.includes(required)) {
      findings.push(`${relative(parityPath)} must consume the native JSON command catalog (${required})`);
    }
  }
  const cliManifestPath = path.join(workspaceRoot, "crates/runx-cli/Cargo.toml");
  const cliManifest = existsSync(cliManifestPath) ? readFileSync(cliManifestPath, "utf8") : "";
  if (!/features\s*=\s*\[[^\]]*"a2a"/su.test(cliManifest) && /adapter-a2a/u.test(parity)) {
    findings.push(`${relative(parityPath)} claims A2A parity although runx-cli does not ship the A2A feature`);
  }

  const driftPath = path.join(workspaceRoot, "scripts/check-command-drift.mjs");
  const drift = existsSync(driftPath) ? readFileSync(driftPath, "utf8") : "";
  for (const token of ["command_spec/catalog.rs", "fixtures/cli-parity/commands.json", "matchAll(/CommandSpec"]) {
    if (drift.includes(token)) {
      findings.push(`${relative(driftPath)} must not parse or mirror the native command registry`);
    }
  }

  const packagePath = path.join(workspaceRoot, "package.json");
  const packageSource = existsSync(packagePath) ? readFileSync(packagePath, "utf8") : "";
  if (/fixtures:cli-help:check|check-help-coverage|canonical-only/u.test(packageSource)) {
    findings.push(`${relative(packagePath)} retains a redundant CLI help/parity validation path`);
  }

  const cutoverPath = path.join(workspaceRoot, "scripts/check-rust-cli-cutover.ts");
  const cutover = existsSync(cutoverPath) ? readFileSync(cutoverPath, "utf8") : "";
  if (/noAliases|no-aliases|inspectCanonicalMatrix/u.test(cutover)) {
    findings.push(`${relative(cutoverPath)} retains a second alias registry over native command help`);
  }
}

function checkRegistryOwnership() {
  const retiredRegistryPaths = [
    "crates/runx-cli/src/registry/remote_publish/payloads.rs",
    "crates/runx-cli/src/registry/package.rs",
  ];
  for (const relPath of retiredRegistryPaths) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} duplicates runtime-owned registry publish contracts`);
    }
  }
  const cliRegistryPaths = [
    path.join(workspaceRoot, "crates/runx-cli/src/registry.rs"),
    path.join(workspaceRoot, "crates/runx-cli/src/registry"),
  ];
  const files = cliRegistryPaths.flatMap((candidate) => {
    if (!existsSync(candidate)) return [];
    return path.extname(candidate) ? [candidate] : walk(candidate).filter((filePath) => filePath.endsWith(".rs"));
  });
  for (const filePath of files) {
    const source = readFileSync(filePath, "utf8");
    if (/\b(?:reqwest|ureq|isahc|attohttpc)::/u.test(source)) {
      findings.push(`${relative(filePath)} owns registry transport instead of using runx-runtime registry services`);
    }
    if (/\bstruct\s+RegistryClient\b/u.test(source)) {
      findings.push(`${relative(filePath)} declares a parallel registry client`);
    }
    if (/\bstruct\s+HostedSkillPackageFile\b/u.test(source)) {
      findings.push(`${relative(filePath)} duplicates runtime-owned RegistryPackageFile`);
    }
    if (/\b(?:load_validated_skill_package|parse_harness_fixture)\b/u.test(source)) {
      findings.push(`${relative(filePath)} prepares registry packages in the CLI instead of using the runtime publish service`);
    }
    if (/\b(?:canonical_remote_registry_url|PublishPackageView|publish_skill_package|publish_admin_package)\b/u.test(source)) {
      findings.push(`${relative(filePath)} retains registry identity or publish wrappers owned by runx-runtime`);
    }
  }
  const runtimeRegistryPath = path.join(workspaceRoot, "crates/runx-runtime/src/registry.rs");
  const runtimeRegistry = existsSync(runtimeRegistryPath)
    ? readFileSync(runtimeRegistryPath, "utf8")
    : "";
  if (/pub\s+use[^;]*\b(?:HttpRequest|HttpResponse|HttpTransport|DefaultRuntimeHttpTransport)\b/su.test(runtimeRegistry)) {
    findings.push(`${relative(runtimeRegistryPath)} re-exports canonical HTTP transport types through the registry`);
  }
  if (!runtimeRegistry.includes("canonical_registry_url")) {
    findings.push(`${relative(runtimeRegistryPath)} must expose the canonical registry source URL owner`);
  }
  const localRegistryPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/registry/local.rs",
  );
  const localRegistry = existsSync(localRegistryPath)
    ? productionRustSource(readFileSync(localRegistryPath, "utf8"))
    : "";
  if (/\bLocalRegistryClient\b|\bcreate_(?:file_registry_store|local_registry_client)\b|pub\s+fn\s+search_registry\s*\(/u.test(localRegistry)) {
    findings.push(`${relative(localRegistryPath)} retains aliases over the canonical FileRegistryStore`);
  }

  const registryHttpPath = path.join(workspaceRoot, "crates/runx-runtime/src/registry/http.rs");
  const registryHttp = existsSync(registryHttpPath) ? readFileSync(registryHttpPath, "utf8") : "";
  if (/\bfn\s+split_skill_id\s*\(/u.test(registryHttp)) {
    findings.push(`${relative(registryHttpPath)} retains a second registry skill-id parser`);
  }

  const publishPackagePath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/registry/publish_package.rs",
  );
  const publishPackage = existsSync(publishPackagePath)
    ? readFileSync(publishPackagePath, "utf8")
    : "";
  for (const token of [
    "prepare_registry_publish_package",
    "RegistryPublishPackageRequest",
    "run_harness",
  ]) {
    if (!publishPackage.includes(token)) {
      findings.push(`${relative(publishPackagePath)} lacks canonical registry publish owner token ${token}`);
    }
  }
  const harnessInferencePath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/registry/publish_package/files/harness_dependencies.rs",
  );
  if (existsSync(harnessInferencePath)) {
    findings.push(`${relative(harnessInferencePath)} infers harness dependencies from arbitrary values`);
  }
  const publishFilesPath = path.join(
    workspaceRoot,
    "crates/runx-runtime/src/registry/publish_package/files.rs",
  );
  const publishFiles = existsSync(publishFilesPath) ? readFileSync(publishFilesPath, "utf8") : "";
  if (!publishFiles.includes("loaded.package.consumed_files")) {
    findings.push(`${relative(publishFilesPath)} must project parser-owned package material`);
  }
  if (/\b(?:collect_publish_harness_files|is_publishable_package_file)\b/u.test(publishFiles)
      || publishFiles.includes("loaded.package.harness_files")) {
    findings.push(`${relative(publishFilesPath)} retains a second package-membership policy`);
  }
}

function checkHttpTransportOwnership() {
  const cliHostedFacade = path.join(workspaceRoot, "crates/runx-cli/src/public_api.rs");
  if (existsSync(cliHostedFacade)) {
    findings.push(`${relative(cliHostedFacade)} duplicates runtime hosted-API ownership`);
  }
  const runtimeHttpRoot = path.join(workspaceRoot, "crates/runx-runtime/src/http");
  const runtimeHttpModule = path.join(runtimeHttpRoot, "mod.rs");
  const runtimeHttpSource = existsSync(runtimeHttpModule) ? readFileSync(runtimeHttpModule, "utf8") : "";
  if (/\bstruct\s+RuntimeHttpClient\b/u.test(runtimeHttpSource)) {
    findings.push(`${relative(runtimeHttpModule)} retains the unused generic RuntimeHttpClient wrapper`);
  }
  for (const filePath of rustFiles("crates/runx-cli/src")) {
    if (filePath.endsWith("_tests.rs")) continue;
    const source = readFileSync(filePath, "utf8");
    const production = source.split(/\n#\[cfg\(test\)\]\nmod\s+tests\b/u, 1)[0] ?? source;
    if (/\bRuntimeHttp(?:Request|Header)\b|\.send\(\s*(?:HttpRequest|RuntimeHttpRequest)\b/u.test(production)) {
      findings.push(`${relative(filePath)} constructs hosted HTTP requests instead of calling a runtime service`);
    }
  }
  for (const filePath of rustFiles("crates")) {
    if (filePath.startsWith(`${runtimeHttpRoot}${path.sep}`)) continue;
    const source = readFileSync(filePath, "utf8");
    if (/\breqwest::/u.test(source)) {
      findings.push(`${relative(filePath)} bypasses the canonical runtime HTTP transport`);
    }
  }

  const requestOwners = [
    "crates/runx-runtime/src/hosted_api/environment.rs",
    "crates/runx-runtime/src/hosted_api/request.rs",
    "crates/runx-runtime/src/registry/http.rs",
    "crates/runx-runtime/src/adapters/agent_anthropic.rs",
    "crates/runx-runtime/src/tool_catalogs/native/web.rs",
  ];
  const requestOwnerRoots = [
    "crates/runx-runtime/src/http/",
    "crates/runx-runtime/src/tool_catalogs/native/http/",
  ];
  for (const filePath of rustFiles("crates")) {
    const rel = relative(filePath);
    if (rel.includes("/tests/") || rel.endsWith("_tests.rs")) continue;
    const source = productionRustSource(readFileSync(filePath, "utf8"));
    const alias = source.match(/RuntimeHttpRequest\s+as\s+([A-Za-z_][A-Za-z0-9_]*)/u)?.[1];
    const constructsRequest = /\bRuntimeHttpRequest\s*\{/u.test(source)
      || (alias !== undefined && new RegExp(`\\b${escapeRegExp(alias)}\\s*\\{`, "u").test(source));
    if (!constructsRequest) continue;
    const allowed = requestOwners.includes(rel)
      || requestOwnerRoots.some((root) => rel.startsWith(root));
    if (!allowed) {
      findings.push(`${rel} constructs RuntimeHttpRequest outside a transport or named protocol owner`);
    }
  }

  const networkPattern = /\bfetch\s*\(|\bXMLHttpRequest\b|\b(?:https?|axios|undici|got)\.(?:get|request)\s*\(|from\s+["'](?:node:)?https?["']|require\(["'](?:node:)?https?["']\)/u;
  const allowed = new Map([
    [
      "skills/nitrosend/tools/nitrosend/bulk_import/run.mjs",
      "runx-architecture-allow: transient-signed-upload",
    ],
  ]);
  for (const filePath of skillProductionFiles()) {
    const source = readFileSync(filePath, "utf8");
    if (!networkPattern.test(source)) continue;
    const rel = relative(filePath);
    const marker = allowed.get(rel);
    if (!marker || !source.includes(marker)) {
      findings.push(`${rel} implements skill-owned HTTP; use native http.read/query/execute`);
    }
  }
}

function checkExternalAdapterOwnership() {
  for (const relPath of [
    "scripts/lib/external-adapter.mjs",
    "examples/adapter-kit/adapter.mjs",
    "scripts/lib/payment-finality-adapter.mjs",
    "scripts/x402-finality-adapter.mjs",
    "scripts/x402-finality-adapter.manifest.json",
    "scripts/stripe-spt-finality-adapter.mjs",
    "scripts/stripe-spt-finality-adapter.manifest.json",
    "scripts/mpp-tempo-finality-adapter.mjs",
    "scripts/mpp-tempo-finality-adapter.manifest.json",
    "scripts/mpp-fiat-finality-adapter.mjs",
    "scripts/mpp-fiat-finality-adapter.manifest.json",
    "tests/payment-finality-adapters.test.ts",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} duplicates a canonical runtime or extension-adapter owner`);
    }
  }

  const standaloneSidecars = new Map([
    [
      "skills/spend/graph/pay-fulfill-rail/tools/stripe-spt-fulfill-adapter.mjs",
      "runx-architecture-allow: portable-external-adapter-sidecar",
    ],
  ]);
  for (const root of ["examples", "scripts", "skills"]) {
    const absoluteRoot = path.join(workspaceRoot, root);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (!/\.(?:js|mjs|cjs|ts)$/u.test(filePath)) continue;
      const source = readFileSync(filePath, "utf8");
      if (/\bfunction\s+runAdapter\s*\(/u.test(source)) {
        const rel = relative(filePath);
        const marker = standaloneSidecars.get(rel);
        if (!marker || !source.includes(marker)) {
          findings.push(`${rel} hand-builds the external-adapter process protocol`);
        }
      }
    }
  }

  const langChainBridgePath = path.join(workspaceRoot, "packages/langchain/src/index.ts");
  const langChainBridge = existsSync(langChainBridgePath)
    ? readFileSync(langChainBridgePath, "utf8")
    : "";
  if (/\b(?:createLangChainToolCatalogAdapter|LangChainToolCatalogAdapterOptions)\b/u.test(langChainBridge)) {
    findings.push(`${relative(langChainBridgePath)} retains a nonfunctional catalog-adapter compatibility API`);
  }

  const parityGeneratorPath = path.join(workspaceRoot, "scripts/generate-cli-feature-parity.ts");
  const parityGenerator = existsSync(parityGeneratorPath)
    ? readFileSync(parityGeneratorPath, "utf8")
    : "";
  if (/adapter-catalog|runx-runtime catalog adapter/u.test(parityGenerator)) {
    findings.push(`${relative(parityGeneratorPath)} retains the displaced catalog adapter as a parity surface`);
  }
}

function checkAuthoringOwnership() {
  const forbiddenPaths = [
    "packages/authoring",
    "fixtures/scaffold",
    "crates/runx-runtime/src/scaffold.rs",
    "crates/runx-runtime/src/scaffold",
    "crates/runx-cli/src/scaffold.rs",
    "scripts/generate-rust-scaffold-fixtures.ts",
    "scripts/materialize-upstream-skill-binding.mjs",
    "scripts/lib/skill-operator-value.mjs",
    "scripts/audit-skill-operator-value.mjs",
    "scripts/trial-core-skills.mjs",
    "scripts/check-skill-capabilities.mjs",
  ];
  for (const relPath of forbiddenPaths) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} is a retired parallel authoring surface`);
    }
  }

  const projectPath = path.join(workspaceRoot, "crates/runx-cli/src/project.rs");
  const projectSource = existsSync(projectPath) ? readFileSync(projectPath, "utf8") : "";
  for (const token of ["PathBuf::from(\"skill-lab\")", "runner: Some(\"build\".to_owned())"] ) {
    if (!projectSource.includes(token)) {
      findings.push(`${relative(projectPath)} must delegate runx new to skill-lab build`);
    }
  }
}

function checkContractBindingOwnership() {
  const bindings = [
    ["packages/contracts/src/schemas/registry.ts", [
      "registry-binding.schema.json",
      "review-receipt-output.schema.json",
    ]],
    ["packages/contracts/src/schemas/operational-policy.ts", [
      "operational-policy.schema.json",
    ]],
  ];
  for (const [relPath, artifacts] of bindings) {
    const filePath = path.join(workspaceRoot, relPath);
    const source = existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
    for (const artifact of artifacts) {
      if (!source.includes("generatedSchema") || !source.includes(`"${artifact}"`)) {
        findings.push(`${relPath} must consume Rust-owned generated schema ${artifact}`);
      }
    }
    if (/\bType\.(?:Object|Union|Record)\s*\(/u.test(source)) {
      findings.push(`${relPath} reconstructs a Rust-owned wire schema in TypeScript`);
    }
  }
}

function checkGeneratedMirrorOwnership() {
  for (const relPath of [
    "packages/cli/dist",
    "packages/cli/skills",
    "packages/cli/tools",
    "scripts/registry-publish-summary.ts",
    "skills/issue-to-pr/push-outbox/manifest.json",
    "examples/governed-spend/verify.mjs",
    "scripts/generate-runtime-catalog-adapter-oracles.ts",
    "scripts/generate-runtime-mcp-oracles.ts",
    "scripts/generate-a2a-adapter-fixtures.ts",
    "scripts/generate-agent-adapter-fixtures.ts",
    "scripts/runtime-adapter-oracle-checks.ts",
    "scripts/check-runtime-catalog-adapter-oracles.sh",
    "scripts/check-runtime-mcp-oracles.sh",
    "scripts/check-tool-catalog-oracles.sh",
    "dist/packets/spec.normalized-scafld-spec.v1.schema.json",
    "dist/packets/spec.declared-file-context.v1.schema.json",
    "examples/host-protocol/openai.ts",
    "crates/runx-contracts/src/cli.rs",
    "crates/runx-contracts/src/receipts.rs",
    "crates/runx-contracts/src/registry.rs",
    "scripts/payment-bridge-spike.mjs",
    "scripts/settlement-finality.mjs",
    "scripts/check-cli-package-contract.mjs",
    "scripts/check-deterministic-module-platform-evidence.mjs",
    "scripts/check-orchestrator-directory-listings.mjs",
    "scripts/check-runtime-cutover-legacy.mjs",
    "scripts/publish-public-package.mjs",
    "scripts/public-package-utils.mjs",
    "docs/runtime-cutover-inventory.json",
    "docs/core-skill-review-decisions.json",
    "docs/core-skill-trial-results.json",
    "docs/core-skill-provider-trials.json",
    "fixtures/runtime/adapters/a2a",
    "fixtures/runtime/adapters/agent",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} is stale generated or mirrored state without a shipping owner`);
    }
  }
  const releaseWorkflowPath = path.join(workspaceRoot, ".github/workflows/release.yml");
  const releaseWorkflow = existsSync(releaseWorkflowPath)
    ? readFileSync(releaseWorkflowPath, "utf8")
    : "";
  if (releaseWorkflow.includes(".scafld/")) {
    findings.push(`${relative(releaseWorkflowPath)} depends on ignored scafld execution state`);
  }
  const skillRoot = path.join(workspaceRoot, "skills");
  for (const filePath of existsSync(skillRoot) ? walk(skillRoot) : []) {
    if (filePath.split(path.sep).includes(".runx")) {
      findings.push(`${relative(filePath)} is generated local runtime state inside a skill package`);
    }
  }

  const coreReviewPath = path.join(workspaceRoot, "docs/core-skill-review.md");
  const coreReview = existsSync(coreReviewPath) ? readFileSync(coreReviewPath, "utf8") : "";
  for (const retired of ["tool:spec.normalize_scafld_frontmatter", "tool:spec.read_declared_files"]) {
    if (coreReview.includes(retired)) {
      findings.push(`${relative(coreReviewPath)} advertises retired capability ${retired}`);
    }
  }
}

function checkRetiredRuntimeSurfaces() {
  for (const relPath of [
    "crates/runx-runtime/src/adapters/catalog.rs",
    "crates/runx-runtime/src/adapters/http.rs",
    "fixtures/runtime/adapters/catalog",
    "fixtures/parser/tool-manifests/catalog-tool-json.json",
    "examples/http-tool-catalog",
    "examples/orchestrator-webhooks",
    "tools/orchestrators/n8n_handoff",
    "tools/orchestrators/zapier_handoff",
    "scripts/check-orchestrator-webhook-templates.mjs",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} is a retired parallel runtime surface`);
    }
  }
  for (const root of ["tools", "examples", "skills"]) {
    const absoluteRoot = path.join(workspaceRoot, root);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (path.basename(filePath) !== "manifest.json") continue;
      let manifest;
      try {
        manifest = JSON.parse(readFileSync(filePath, "utf8"));
      } catch {
        continue;
      }
      if (["http", "catalog"].includes(manifest?.source?.type)) {
        findings.push(
          `${relative(filePath)} retains retired source.type ${manifest.source.type}; use a graph tool step`,
        );
      }
    }
  }
  const toolParserPath = path.join(workspaceRoot, "crates/runx-parser/src/tool.rs");
  const toolParser = existsSync(toolParserPath) ? readFileSync(toolParserPath, "utf8") : "";
  if (/normalize_tool_manifest_shape|"catalog"\s*\|/u.test(toolParser)) {
    findings.push(`${relative(toolParserPath)} retains retired tool-source normalization or admission`);
  }
  const toolContractPath = path.join(
    workspaceRoot,
    "packages/contracts/src/schemas/tool-manifest.ts",
  );
  const toolContract = existsSync(toolContractPath) ? readFileSync(toolContractPath, "utf8") : "";
  if (/ToolManifestHttpSourceContract|"catalog"|"http"|catalog_ref/u.test(toolContract)) {
    findings.push(`${relative(toolContractPath)} drifts from the generated canonical tool-source schema`);
  }
  for (const filePath of rustFiles("crates/runx-runtime/src")) {
    const source = readFileSync(filePath, "utf8");
    if (/\bCatalogAdapter\b|\badapters::catalog\b/u.test(source)) {
      findings.push(`${relative(filePath)} retains the displaced catalog adapter`);
    }
  }
  for (const [relPath, tokens] of [
    ["crates/runx-runtime/src/execution/runner/steps.rs", ["tool_catalogs::dispatch::ToolDispatchRequest", "tool_catalogs::dispatch::dispatch_tool"]],
    ["crates/runx-runtime/src/adapters/agent_tools.rs", ["tool_catalogs::dispatch", "dispatch_tool"]],
  ]) {
    const filePath = path.join(workspaceRoot, relPath);
    const source = existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
    for (const token of tokens) {
      if (!source.includes(token)) {
        findings.push(`${relPath} must use the canonical tool dispatcher (${token})`);
      }
    }
  }

  const runnerFiles = [
    ...rustFiles("crates/runx-runtime/src/execution/runner"),
    path.join(workspaceRoot, "crates/runx-runtime/src/execution/runner.rs"),
  ].filter(existsSync);
  for (const filePath of runnerFiles) {
    const source = readFileSync(filePath, "utf8");
    for (const pattern of [
      /\bpayment_supervisor\b/u,
      /\b(?:crate|runx_runtime)::payment::state\b/u,
      /\b(?:use\s+)?crate::payment::/u,
    ]) {
      if (pattern.test(source)) {
        findings.push(`${relative(filePath)} retains retired payment orchestration ${pattern}`);
      }
    }
  }

  const domainTokens = new Set(["payment", "settlement", "spend", "x402", "rail"]);
  for (const root of [
    "crates/runx-runtime/src",
    "crates/runx-core/src",
    "crates/runx-contracts/src",
  ]) {
    for (const filePath of rustFiles(root)) {
      const lines = readFileSync(filePath, "utf8").split(/\r?\n/u);
      lines.forEach((line, index) => {
        for (const token of line.matchAll(/[A-Za-z_][A-Za-z0-9_]*/gu)) {
          const banned = splitIdentifierParts(token[0]).find((part) => domainTokens.has(part));
          if (banned) {
            findings.push(`${relative(filePath)}:${index + 1} contains domain token '${banned}' in '${token[0]}'`);
          }
        }
      });
    }
  }

  for (const relPath of [
    "crates/runx-runtime/src/execution/target_runner.rs",
    "crates/runx-runtime/src/execution/target_runner",
    "crates/runx-runtime/src/post_merge_observer.rs",
    "crates/runx-runtime/src/post_merge_observer",
    "crates/runx-contracts/src/target_runner.rs",
    "crates/runx-contracts/src/target_runner",
    "crates/runx-contracts/src/post_merge_observer.rs",
    "crates/runx-contracts/src/post_merge_observer",
  ]) {
    if (existsSync(path.join(workspaceRoot, relPath))) {
      findings.push(`${relPath} reintroduces retired provider orchestration`);
    }
  }

  const providerClientMarkers = [/\breqwest\b/u, /\bapi\.github\.com\b/u, /\bGITHUB_TOKEN\b/u, /\bbearer_auth\b/u];
  for (const root of ["crates/runx-runtime/src/adapters", "crates/runx-runtime/src/outbox_provider"]) {
    for (const filePath of rustFiles(root)) {
      const source = readFileSync(filePath, "utf8");
      const marker = providerClientMarkers.find((pattern) => pattern.test(source));
      if (marker) {
        findings.push(`${relative(filePath)} contains outbound GitHub provider client marker ${marker}`);
      }
    }
  }

  const retiredWirePatterns = [
    /PaymentAuthorityBounds/u,
    /PaymentCredentialForm/u,
    /\bbounds\.payment\b/u,
    /max_spend_usd/u,
    /max_per_call_minor/u,
    /max_per_run_minor/u,
    /max_per_period_minor/u,
    /payment_single_use_spend/u,
    /single_use_spend_capability/u,
    /ProofKind::PaymentRail/u,
    /"payment_rail"/u,
    /\bpayment_rail\b/u,
    /EffectSettlementReceipt/u,
    /\beffect_settlement\b/u,
    /\beffect-settlement\b/u,
    /\bpayment_required\b/u,
    /payment_rail_packet/u,
    /runx\.payment\.rail\.v1/u,
    /\bquote_required\b/u,
    /\breservation_required\b/u,
    /\bcredential_form\b/u,
    /\bsingle_use_spend\b/u,
    /resource_family:\s*payment/u,
    /"resource_family"\s*:\s*"payment"/u,
  ];
  const roots = ["crates/runx-contracts", "packages/contracts/src", "schemas", "fixtures", "skills", "examples", "scripts", "docs"];
  const extensions = new Set([".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json", ".yaml", ".yml", ".md"]);
  for (const root of roots) {
    const absoluteRoot = path.join(workspaceRoot, root);
    for (const filePath of existsSync(absoluteRoot) ? walk(absoluteRoot) : []) {
      if (!extensions.has(path.extname(filePath)) || filePath === currentScriptPath) continue;
      const source = readFileSync(filePath, "utf8");
      const pattern = retiredWirePatterns.find((candidate) => candidate.test(source));
      if (pattern) {
        findings.push(`${relative(filePath)} contains retired generic-contract wire name ${pattern}`);
      }
    }
  }
}

function checkServiceBoundary() {
  const roots = [
    "crates/runx-runtime/src/adapters",
    "crates/runx-runtime/src/execution",
  ];
  const forbidden = [
    /\bRuntimeReceiptSignatureConfig::from_env\b/u,
    /\bLocalReceiptStore::new\b/u,
    /\bresolve_receipt_path\s*\(/u,
    /\bprepare_process_sandbox\s*\(/u,
    /\bprepare_mcp_process_sandbox\s*\(/u,
    /\bstd::env::(?:var|vars)\s*\(/u,
  ];
  for (const root of roots) {
    for (const filePath of rustFiles(root)) {
      const source = readFileSync(filePath, "utf8");
      for (const pattern of forbidden) {
        if (pattern.test(source)) {
          findings.push(`${relative(filePath)} still constructs env, receipts, or sandbox state outside runtime services`);
        }
      }
    }
  }
}

function checkExecutionSplit() {
  const stepsPath = path.join(workspaceRoot, "crates/runx-runtime/src/execution/runner/steps.rs");
  if (!existsSync(stepsPath)) {
    return;
  }
  const source = readFileSync(stepsPath, "utf8");
  const forbidden = [
    /\bstep_receipt_with\b/u,
    /\brequest_approval\b/u,
    /\bSkillAdapter::invoke\b/u,
    /\bresolve_inputs\b/u,
  ];
  for (const pattern of forbidden) {
    if (pattern.test(source)) {
      findings.push(`${relative(stepsPath)} still contains mixed runner responsibility token ${pattern}`);
    }
  }
}

function checkProjectionHotPaths() {
  const runtimeRoot = path.join(workspaceRoot, "crates/runx-runtime/src");
  const compactIndexFound = rustFiles("crates/runx-runtime/src").some((filePath) => {
    const source = readFileSync(filePath, "utf8");
    return /\bstruct\s+\w*(?:Id)?Interner\b/u.test(source)
      || /\bstruct\s+\w*(?:Step)?PositionIndex\b[\s\S]*?\bpositions:\s*BTreeMap<String,\s*usize>/u.test(source);
  });
  if (!compactIndexFound) {
    findings.push(`${relative(runtimeRoot)} has no runtime-local id interner or compact position index for hot execution/projection paths`);
  }

  const cloneBudget = new Map([
    ["crates/runx-runtime/src/execution/graph_index.rs", 8],
    ["crates/runx-runtime/src/execution/output_projection.rs", 8],
  ]);
  for (const [relPath, maxClones] of cloneBudget) {
    const filePath = path.join(workspaceRoot, relPath);
    if (!existsSync(filePath)) {
      continue;
    }
    const count = countMatches(readFileSync(filePath, "utf8"), /\.clone\s*\(/gu);
    if (count > maxClones) {
      findings.push(`${relPath} has ${count} .clone() calls, above budget ${maxClones}`);
    }
  }
}

function checkSessionPooling() {
  for (const filePath of rustFiles("crates/runx-runtime/src")) {
    const source = readFileSync(filePath, "utf8");
    if (/\b(?:cli.*pool|pool.*cli|user command pool|pooled.*Command|CommandPool)\b/iu.test(source)) {
      findings.push(`${relative(filePath)} appears to pool arbitrary CLI/user commands`);
    }
  }
  const mcpTransportPath = path.join(workspaceRoot, "crates/runx-runtime/src/adapters/mcp/transport.rs");
  const mcpTransport = existsSync(mcpTransportPath) ? readFileSync(mcpTransportPath, "utf8") : "";
  for (const pattern of [
    /\bstruct\s+McpSessionManager\b/u,
    /\bstruct\s+McpSessionKey\b/u,
    /\breset_session_pool\b/u,
    /\bspawned_process_count\b/u,
  ]) {
    if (!pattern.test(mcpTransport)) {
      findings.push(`${relative(mcpTransportPath)} lacks required MCP session-pooling token ${pattern}`);
    }
  }
  const perfHarnessPath = path.join(workspaceRoot, "scripts/runtime-throughput.mjs");
  const perfHarness = existsSync(perfHarnessPath) ? readFileSync(perfHarnessPath, "utf8") : "";
  if (!/\brunx-mcp-session-probe\b/u.test(perfHarness) || /mcp_session_reuse[\s\S]{0,400}source:\s*"node"/u.test(perfHarness)) {
    findings.push(`${relative(perfHarnessPath)} must measure MCP session workloads through the Rust MCP session probe`);
  }
}

function rustFiles(root) {
  const absoluteRoot = path.join(workspaceRoot, root);
  if (!existsSync(absoluteRoot)) {
    return [];
  }
  return walk(absoluteRoot).filter((filePath) => filePath.endsWith(".rs"));
}

function productionRustSource(source) {
  return source.split(/\n#\[cfg\(test\)\]\s*\nmod\s+tests\b/u, 1)[0] ?? source;
}

function skillProductionFiles() {
  const skillRoot = path.join(workspaceRoot, "skills");
  if (!existsSync(skillRoot)) return [];
  const extensions = new Set([".js", ".mjs", ".cjs", ".ts"]);
  return walk(skillRoot).filter((filePath) => {
    if (!extensions.has(path.extname(filePath))) return false;
    const segments = filePath.split(path.sep);
    if (segments.some((segment) => ["fixtures", "harness", ".runx"].includes(segment))) return false;
    return !/\.(?:test|spec)\.(?:js|mjs|cjs|ts)$/u.test(filePath);
  });
}

function walk(directory) {
  const entries = readdirSync(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.name === "target") {
      continue;
    }
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...walk(entryPath));
    } else if (entry.isFile()) {
      files.push(entryPath);
    }
  }
  return files;
}

function countMatches(source, pattern) {
  return [...source.matchAll(pattern)].length;
}

function splitIdentifierParts(token) {
  return token
    .replace(/([a-z0-9])([A-Z])/gu, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/gu, "$1_$2")
    .toLowerCase()
    .split(/_+/u)
    .filter(Boolean);
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

function relative(filePath) {
  return path.relative(workspaceRoot, filePath);
}
