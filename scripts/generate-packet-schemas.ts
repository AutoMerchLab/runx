import { readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import YAML from "yaml";

type JsonObject = Record<string, unknown>;

interface PacketContract {
  readonly packetId: string;
  readonly source: string;
  readonly schema: JsonObject;
}

interface ExistingPacketSchema {
  readonly path: string;
  readonly generated: boolean;
  readonly schema: JsonObject;
}

const workspaceRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const skillsRoot = path.join(workspaceRoot, "skills");
const packetRoot = path.join(workspaceRoot, "dist", "packets");
const check = process.argv.includes("--check");
const contracts = new Map<string, PacketContract>();
const manualContracts: PacketContract[] = [];
const declarations = new Map<string, string>();
const existingById = await existingSchemas();

for (const profilePath of await findProfiles(skillsRoot)) {
  const profile = YAML.parse(await readFile(profilePath, "utf8")) as unknown;
  collectContracts(profile, path.relative(workspaceRoot, profilePath), "root");
}

const manualSchemaFindings: string[] = [];
for (const contract of manualContracts) {
  const existing = existingById.get(contract.packetId);
  if (!existing) throw new Error(`manual packet schema '${contract.packetId}' was not found`);
  manualSchemaFindings.push(
    ...structuralFloorFindings(existing.schema, contract.schema, contract.packetId, contract.source),
  );
}
const manualContractsByPacket = new Map<string, PacketContract[]>();
for (const contract of manualContracts) {
  const packetContracts = manualContractsByPacket.get(contract.packetId) ?? [];
  packetContracts.push(contract);
  manualContractsByPacket.set(contract.packetId, packetContracts);
}
for (const [packetId, packetContracts] of manualContractsByPacket) {
  const requiredSets = packetContracts.map((contract) => new Set(stringArray(contract.schema.required)));
  const shapes = new Set(requiredSets.map((required) => [...required].sort().join("\u0000")));
  if (shapes.size < 2) continue;
  const commonRequired = requiredSets.slice(1).reduce(
    (common, required) => new Set([...common].filter((field) => required.has(field))),
    new Set(requiredSets[0] ?? []),
  );
  const existing = existingById.get(packetId);
  if (!existing) throw new Error(`manual packet schema '${packetId}' was not found`);
  for (const field of schemaView(existing.schema, existing.schema, new Set()).required) {
    if (!commonRequired.has(field)) {
      manualSchemaFindings.push(
        `manual packet schema '${packetId}' unconditionally requires root property '${field}', but its X.yaml bindings use incompatible envelope shapes`,
      );
    }
  }
}
if (manualSchemaFindings.length > 0) {
  throw new Error(`manual packet schemas conflict with X.yaml output contracts:\n${manualSchemaFindings.join("\n")}`);
}

for (const contract of [...contracts.values()].sort((left, right) => left.packetId.localeCompare(right.packetId))) {
  const existing = existingById.get(contract.packetId);
  if (existing && !existing.generated) continue;
  const filePath = existing?.path ?? path.join(packetRoot, `${packetFileName(contract.packetId)}.schema.json`);
  const document = `${JSON.stringify({
    $schema: "https://json-schema.org/draft/2020-12/schema",
    $id: packetSchemaId(contract.packetId),
    "x-runx-packet-id": contract.packetId,
    "x-runx-generated-from": contract.source,
    ...contract.schema,
  }, null, 2)}\n`;
  if (check) {
    const current = await readFile(filePath, "utf8").catch(() => undefined);
    if (current !== document) {
      throw new Error(`packet schema is missing or stale: ${path.relative(workspaceRoot, filePath)}`);
    }
  } else {
    await writeFile(filePath, document, "utf8");
  }
}

const missing = [...declarations.keys()].filter(
  (packetId) => !existingById.has(packetId) && !contracts.has(packetId),
);
if (missing.length > 0) {
  throw new Error(`packet declarations have no schema contract: ${missing.join(", ")}`);
}
console.log(`${check ? "checked" : "generated"} ${declarations.size} packet contracts`);

function collectContracts(value: unknown, profile: string, location: string): void {
  if (Array.isArray(value)) {
    value.forEach((child, index) => collectContracts(child, profile, `${location}.${index}`));
    return;
  }
  if (!isRecord(value)) return;
  const execution = isRecord(value.run) && typeof value.run.type === "string"
    ? value.run
    : isRecord(value.source) && typeof value.source.type === "string"
      ? value.source
      : value;
  const type = execution.type;
  const outputs = isRecord(execution.outputs)
    ? execution.outputs
    : isRecord(value.outputs)
      ? value.outputs
      : undefined;
  const artifacts = isRecord(value.artifacts)
    ? value.artifacts
    : isRecord(execution.artifacts)
      ? execution.artifacts
      : undefined;
  if (type === "agent" || type === "agent-task") {
    if (!outputs || Object.keys(outputs).length === 0) {
      throw new Error(`${profile}#${location} agent runner has no declared outputs`);
    }
  }
  if (artifacts) {
    const source = `${profile}#${location}`;
    collectPacketDeclarations(artifacts, source);
    if (outputs && Object.keys(outputs).length > 0) {
      collectArtifactContracts(artifacts, outputs, source);
    }
  }
  for (const [key, child] of Object.entries(value)) {
    collectContracts(child, profile, `${location}.${key}`);
  }
}

function collectPacketDeclarations(artifacts: JsonObject, source: string): void {
  const packetIds = [nonEmptyString(artifacts.packet)];
  if (isRecord(artifacts.packets)) {
    packetIds.push(...Object.values(artifacts.packets).map(nonEmptyString));
  }
  for (const packetId of packetIds) {
    if (!packetId) continue;
    const existing = declarations.get(packetId);
    if (!existing) declarations.set(packetId, source);
  }
}

function collectArtifactContracts(
  artifacts: JsonObject,
  outputs: JsonObject,
  source: string,
): void {
  const wrapAs = nonEmptyString(artifacts.wrap_as);
  const packet = nonEmptyString(artifacts.packet);
  if (packet) {
    if (!wrapAs) throw new Error(`${source} packet requires wrap_as`);
    register({
      packetId: packet,
      source,
      schema: objectSchema(outputs),
    });
  }
  if (!isRecord(artifacts.packets)) return;
  for (const [output, packetValue] of Object.entries(artifacts.packets)) {
    const packetId = nonEmptyString(packetValue);
    if (!packetId) throw new Error(`${source} packets.${output} must be a packet id`);
    if (!(output in outputs)) throw new Error(`${source} packets.${output} has no matching output declaration`);
    register({ packetId, source, schema: outputSchema(outputs[output]) });
  }
}

function register(contract: PacketContract): void {
  if (existingById.get(contract.packetId)?.generated === false) {
    manualContracts.push(contract);
    if (!contracts.has(contract.packetId)) contracts.set(contract.packetId, contract);
    return;
  }
  const existing = contracts.get(contract.packetId);
  if (existing && JSON.stringify(existing.schema) !== JSON.stringify(contract.schema)) {
    throw new Error(`packet '${contract.packetId}' has conflicting X.yaml output contracts`);
  }
  if (!existing) contracts.set(contract.packetId, contract);
}

function structuralFloorFindings(
  actual: JsonObject,
  floor: JsonObject,
  packetId: string,
  source: string,
): readonly string[] {
  const findings: string[] = [];
  const actualView = schemaView(actual, actual, new Set());
  const floorType = nonEmptyString(floor.type);
  if (floorType && actualView.type !== floorType) {
    findings.push(
      `manual packet schema '${packetId}' for ${source} must constrain the root to type '${floorType}'`,
    );
  }
  const required = stringArray(floor.required);
  for (const field of required) {
    const expected = isRecord(floor.properties) ? floor.properties[field] : undefined;
    const declaredType = isRecord(expected) ? nonEmptyString(expected.type) : undefined;
    const actualProperty = actualView.properties.get(field);
    if (!actualProperty) {
      findings.push(
        `manual packet schema '${packetId}' for ${source} must declare root property '${field}'`,
      );
      continue;
    }
    const actualType = schemaView(actualProperty, actual, new Set()).type;
    if (declaredType && actualType !== declaredType) {
      findings.push(
        `manual packet schema '${packetId}' for ${source} must type root property '${field}' as '${declaredType}'`,
      );
    }
  }
  return findings;
}

function schemaView(
  schema: JsonObject,
  root: JsonObject,
  visitedRefs: Set<string>,
): {
  readonly type?: string;
  readonly required: Set<string>;
  readonly properties: Map<string, JsonObject>;
} {
  let type = nonEmptyString(schema.type);
  const required = new Set(stringArray(schema.required));
  const properties = new Map<string, JsonObject>();
  if (isRecord(schema.properties)) {
    for (const [name, value] of Object.entries(schema.properties)) {
      if (isRecord(value)) properties.set(name, value);
    }
  }
  const branches: JsonObject[] = [];
  const ref = nonEmptyString(schema.$ref);
  if (ref?.startsWith("#/") && !visitedRefs.has(ref)) {
    const resolved = resolveLocalRef(root, ref);
    if (resolved) {
      visitedRefs.add(ref);
      branches.push(resolved);
    }
  }
  if (Array.isArray(schema.allOf)) {
    branches.push(...schema.allOf.filter(isRecord));
  }
  for (const branch of branches) {
    const view = schemaView(branch, root, new Set(visitedRefs));
    type ??= view.type;
    for (const field of view.required) required.add(field);
    for (const [name, value] of view.properties) properties.set(name, value);
  }
  return { type, required, properties };
}

function resolveLocalRef(root: JsonObject, ref: string): JsonObject | undefined {
  let value: unknown = root;
  for (const encoded of ref.slice(2).split("/")) {
    if (!isRecord(value)) return undefined;
    const segment = encoded.replace(/~1/gu, "/").replace(/~0/gu, "~");
    value = value[segment];
  }
  return isRecord(value) ? value : undefined;
}

function stringArray(value: unknown): readonly string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function objectSchema(outputs: JsonObject): JsonObject {
  const required = Object.entries(outputs)
    .filter(([, declaration]) => outputIsRequired(declaration))
    .map(([name]) => name)
    .sort();
  return {
    type: "object",
    required,
    properties: Object.fromEntries(
      Object.entries(outputs)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([name, declaration]) => [name, outputSchema(declaration)]),
    ),
    additionalProperties: false,
  };
}

function outputIsRequired(declaration: unknown): boolean {
  return !isRecord(declaration) || declaration.required !== false;
}

function outputSchema(declaration: unknown): JsonObject {
  const type = typeof declaration === "string"
    ? declaration
    : isRecord(declaration) && typeof declaration.type === "string"
      ? declaration.type
      : "json";
  switch (type) {
    case "string": return { type: "string" };
    case "number": return { type: "number" };
    case "integer": return { type: "integer" };
    case "boolean": return { type: "boolean" };
    case "array": return { type: "array" };
    case "object": return { type: "object" };
    case "json": return {};
    default: throw new Error(`unsupported agent output type '${type}'`);
  }
}

async function existingSchemas(): Promise<Map<string, ExistingPacketSchema>> {
  const schemas = new Map<string, ExistingPacketSchema>();
  for (const entry of (await readdir(packetRoot)).filter((name) => name.endsWith(".json")).sort()) {
    const filePath = path.join(packetRoot, entry);
    const value = JSON.parse(await readFile(filePath, "utf8")) as JsonObject;
    const packetId = nonEmptyString(value["x-runx-packet-id"]);
    if (!packetId) continue;
    if (schemas.has(packetId)) throw new Error(`duplicate packet schema id '${packetId}'`);
    schemas.set(packetId, {
      path: filePath,
      generated: typeof value["x-runx-generated-from"] === "string",
      schema: value,
    });
  }
  return schemas;
}

async function findProfiles(directory: string): Promise<readonly string[]> {
  const profiles: string[] = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) profiles.push(...await findProfiles(entryPath));
    else if (entry.isFile() && entry.name === "X.yaml") profiles.push(entryPath);
  }
  return profiles.sort();
}

function packetFileName(packetId: string): string {
  return packetId.replace(/[^a-zA-Z0-9]+/g, ".").replace(/^\.+|\.+$/g, "");
}

function packetSchemaId(packetId: string): string {
  const segments = packetId.split(".").filter(Boolean);
  if (segments[0] === "runx") segments.shift();
  return `https://schemas.runx.ai/runx/${segments.join("/")}.json`;
}

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function isRecord(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
