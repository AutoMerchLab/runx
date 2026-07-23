import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const workspaceRoot = path.resolve(fileURLToPath(new URL("../..", import.meta.url)));
const defaultBinary = path.join(
  workspaceRoot,
  "crates",
  "target",
  "debug",
  process.platform === "win32" ? "runx.exe" : "runx",
);
const cache = new Map();

export function resolveNativeRunxBinary(env = process.env) {
  const configured = env.RUNX_RUST_CLI_BIN;
  const candidate = configured ?? defaultBinary;
  const resolved = path.isAbsolute(candidate) ? candidate : path.resolve(workspaceRoot, candidate);
  if (!existsSync(resolved)) {
    throw new Error(
      `native Runx parser is missing at ${resolved}; build it with cargo build --manifest-path crates/Cargo.toml -p runx-cli --bin runx`,
    );
  }
  return resolved;
}

export function evaluateParserRequests(inputs, env = process.env) {
  if (!Array.isArray(inputs) || inputs.length === 0) return [];
  return invokeNativeParser({ inputs }, inputs.length, env);
}

export function evaluateParserRequestResults(inputs, env = process.env) {
  if (!Array.isArray(inputs) || inputs.length === 0) return [];
  return invokeNativeParser({ inputs, returnErrors: true }, inputs.length, env);
}

function invokeNativeParser(document, expectedLength, env) {
  const request = JSON.stringify(document);
  const cached = cache.get(request);
  if (cached) return cached;

  const result = spawnSync(
    resolveNativeRunxBinary(env),
    ["parser", "eval", "--input", "-", "--json"],
    {
      cwd: workspaceRoot,
      encoding: "utf8",
      env,
      input: request,
      maxBuffer: 32 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(parserFailureMessage(result.stdout, result.stderr));
  }

  const envelope = JSON.parse(result.stdout);
  const values = envelope?.result?.value;
  if (envelope?.status !== "success" || !Array.isArray(values) || values.length !== expectedLength) {
    throw new Error("native Runx parser returned an invalid batch response");
  }
  cache.set(request, values);
  if (Array.isArray(document.inputs) && document.returnErrors !== true) {
    document.inputs.forEach((input, index) => {
      cache.set(JSON.stringify({ inputs: [input] }), [values[index]]);
    });
  }
  return values;
}

export function validateSkillMarkdownBatch(markdowns, env = process.env) {
  return evaluateParserRequests(
    markdowns.map((markdown) => ({ kind: "parser.validateSkillMarkdown", markdown })),
    env,
  );
}

export function validateRunnerManifestYamlBatch(documents, env = process.env) {
  return evaluateParserRequests(
    documents.map((yaml) => ({ kind: "parser.validateRunnerManifestYaml", yaml })),
    env,
  );
}

export function validateGraphYamlBatch(documents, env = process.env) {
  return evaluateParserRequests(
    documents.map((yaml) => ({ kind: "parser.validateGraphYaml", yaml })),
    env,
  );
}

export function validateHarnessFixtureYamlBatch(documents, env = process.env) {
  return evaluateParserRequests(
    documents.map((yaml) => ({ kind: "parser.validateHarnessFixtureYaml", yaml })),
    env,
  );
}

export function parsePacketSchemaDocumentsBatch(documents, env = process.env) {
  return evaluateParserRequests(
    documents.map(({ path: documentPath, source }) => ({
      kind: "parser.parsePacketSchemaDocument",
      path: documentPath,
      source,
    })),
    env,
  );
}

export function validateSkillMarkdown(markdown, env = process.env) {
  return validateSkillMarkdownBatch([markdown], env)[0];
}

export function validateRunnerManifestYaml(yaml, env = process.env) {
  return validateRunnerManifestYamlBatch([yaml], env)[0];
}

export function validateGraphYaml(yaml, env = process.env) {
  return validateGraphYamlBatch([yaml], env)[0];
}

export function validateHarnessFixtureYaml(yaml, env = process.env) {
  return validateHarnessFixtureYamlBatch([yaml], env)[0];
}

export function parsePacketSchemaDocument(document, env = process.env) {
  return parsePacketSchemaDocumentsBatch([document], env)[0];
}

function parserFailureMessage(stdout, stderr) {
  try {
    const envelope = JSON.parse(stdout);
    if (typeof envelope?.error?.message === "string") return envelope.error.message;
    if (typeof envelope?.message === "string") return envelope.message;
  } catch {
    // Fall through to the bounded process output.
  }
  return String(stderr || stdout || "native Runx parser failed").trim();
}
