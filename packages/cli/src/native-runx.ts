import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { errorMessage, firstNonEmpty, parsePositiveInt } from "./cli-util.js";

const DEFAULT_NATIVE_RUNX_TIMEOUT_MS = 300_000;
const DEFAULT_NATIVE_RUNX_OUTPUT_LIMIT_BYTES = 8_388_608;
const CLI_PACKAGE_NAME = "@runxhq/cli";

const requireFromCli = createRequire(import.meta.url);

export interface NativeRunxExitResult {
  readonly status: number | null;
  readonly signal: NodeJS.Signals | null;
}

export interface NativeRunxProcessResult extends NativeRunxExitResult {
  readonly stdout: string;
  readonly stderr: string;
}

export interface NativeRunxOptions {
  readonly env: NodeJS.ProcessEnv;
  readonly cwd?: string;
  readonly timeoutMs?: number;
  readonly stderr?: NodeJS.WritableStream;
}

export interface NativeRunxStreamOptions extends NativeRunxOptions {
  readonly stdout: NodeJS.WritableStream;
  readonly stderr: NodeJS.WritableStream;
}

export async function runNativeRunxJson(
  args: readonly string[],
  options: NativeRunxOptions,
): Promise<unknown> {
  const result = await runNativeRunx(args, options);
  if (result.status !== 0) {
    throw nativeRunxError(args, result);
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`native runx ${args.join(" ")} returned invalid JSON: ${errorMessage(error)}`);
  }
}

export async function runNativeRunx(
  args: readonly string[],
  options: NativeRunxOptions,
): Promise<NativeRunxProcessResult> {
  const timeoutMs = nativeRunxTimeoutMs(options);
  const maxOutputBytes = parsePositiveInt(options.env.RUNX_RUST_CLI_OUTPUT_LIMIT_BYTES)
    ?? DEFAULT_NATIVE_RUNX_OUTPUT_LIMIT_BYTES;
  return await spawnNativeRunx({
    command: resolveNativeRunxBinary(options.env),
    args,
    cwd: nativeRunxCwd(options),
    env: nativeRunxEnv(options.env),
    timeoutMs,
    maxOutputBytes,
    stderr: options.stderr,
  });
}

export async function streamNativeRunx(
  args: readonly string[],
  options: NativeRunxStreamOptions,
): Promise<NativeRunxExitResult> {
  return await spawnStreamingNativeRunx({
    command: resolveNativeRunxBinary(options.env),
    args,
    cwd: nativeRunxCwd(options),
    env: nativeRunxEnv(options.env),
    timeoutMs: nativeRunxTimeoutMs(options),
    stdout: options.stdout,
    stderr: options.stderr,
  });
}

export function resolveNativeRunxBinary(env: NodeJS.ProcessEnv): string {
  const override = env.RUNX_DEV_RUST_CLI_BIN;
  if (override) {
    if (!path.isAbsolute(override)) {
      throw new Error(`RUNX_DEV_RUST_CLI_BIN must be an absolute path: ${override}`);
    }
    if (existsSync(override)) {
      return override;
    }
    throw new Error(`RUNX_DEV_RUST_CLI_BIN does not exist: ${override}`);
  }
  const verifiedBinary = resolveVerifiedPlatformNativeRunxBinary();
  if (verifiedBinary) {
    return verifiedBinary;
  }
  throw new Error(
    `runx native package could not be verified for ${process.platform}-${process.arch}; set RUNX_DEV_RUST_CLI_BIN to an absolute development binary path.`,
  );
}

interface SpawnNativeRunxOptions {
  readonly command: string;
  readonly args: readonly string[];
  readonly cwd: string;
  readonly env: NodeJS.ProcessEnv;
  readonly timeoutMs: number;
  readonly maxOutputBytes: number;
  readonly stderr?: NodeJS.WritableStream;
}

export function spawnNativeRunx(options: SpawnNativeRunxOptions): Promise<NativeRunxProcessResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(options.command, options.args, {
      cwd: options.cwd,
      env: options.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    if (!child.stdout || !child.stderr) {
      reject(new Error("failed to open native runx stdout/stderr pipes."));
      return;
    }
    let settled = false;
    let timedOut = false;
    let stdout = "";
    let stderr = "";
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let outputLimitExceeded: string | undefined;
    let killTimer: NodeJS.Timeout | undefined;

    const terminate = () => {
      child.kill("SIGTERM");
      killTimer = setTimeout(() => {
        if (settled) return;
        settled = true;
        child.kill("SIGKILL");
        reject(new Error(outputLimitExceeded ?? `native runx ${options.args.join(" ")} timed out after ${options.timeoutMs}ms.`));
      }, 1_000);
    };

    const timer = setTimeout(() => {
      if (settled) return;
      timedOut = true;
      terminate();
    }, options.timeoutMs);

    const clearTimers = () => {
      clearTimeout(timer);
      if (killTimer) clearTimeout(killTimer);
    };

    const appendOutput = (stream: "stdout" | "stderr", chunk: string) => {
      if (outputLimitExceeded) return;
      const chunkBytes = Buffer.byteLength(chunk, "utf8");
      if (stream === "stdout") {
        stdoutBytes += chunkBytes;
        if (stdoutBytes <= options.maxOutputBytes) {
          stdout += chunk;
          return;
        }
      } else {
        stderrBytes += chunkBytes;
        if (stderrBytes <= options.maxOutputBytes) {
          stderr += chunk;
          options.stderr?.write(chunk);
          return;
        }
      }
      outputLimitExceeded = `native runx ${stream} exceeded ${options.maxOutputBytes} bytes.`;
      terminate();
    };

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      appendOutput("stdout", chunk);
    });
    child.stderr.on("data", (chunk: string) => {
      appendOutput("stderr", chunk);
    });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      clearTimers();
      reject(new Error(`failed to spawn native runx '${options.command}': ${error.message}`));
    });
    child.on("close", (status, signal) => {
      if (settled) return;
      settled = true;
      clearTimers();
      if (outputLimitExceeded) {
        reject(new Error(outputLimitExceeded));
        return;
      }
      if (timedOut) {
        reject(new Error(`native runx ${options.args.join(" ")} timed out after ${options.timeoutMs}ms.`));
        return;
      }
      resolve({ status, signal, stdout, stderr });
    });
  });
}

interface SpawnStreamingNativeRunxOptions {
  readonly command: string;
  readonly args: readonly string[];
  readonly cwd: string;
  readonly env: NodeJS.ProcessEnv;
  readonly timeoutMs: number;
  readonly stdout: NodeJS.WritableStream;
  readonly stderr: NodeJS.WritableStream;
}

export function spawnStreamingNativeRunx(options: SpawnStreamingNativeRunxOptions): Promise<NativeRunxExitResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(options.command, options.args, {
      cwd: options.cwd,
      env: options.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    if (!child.stdout || !child.stderr) {
      reject(new Error("failed to open native runx stdout/stderr pipes."));
      return;
    }
    let settled = false;
    let timedOut = false;
    let killTimer: NodeJS.Timeout | undefined;

    const fail = (error: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.kill("SIGTERM");
      killTimer = setTimeout(() => {
        child.kill("SIGKILL");
      }, 1_000);
      reject(error);
    };

    const terminate = () => {
      child.kill("SIGTERM");
      killTimer = setTimeout(() => {
        if (settled) return;
        settled = true;
        child.kill("SIGKILL");
        reject(new Error(`native runx ${options.args.join(" ")} timed out after ${options.timeoutMs}ms.`));
      }, 1_000);
    };

    const timer = setTimeout(() => {
      if (settled) return;
      timedOut = true;
      terminate();
    }, options.timeoutMs);

    const clearTimers = () => {
      clearTimeout(timer);
      if (killTimer) clearTimeout(killTimer);
    };

    const forwardOutput = (
      chunk: Buffer,
      source: NodeJS.ReadableStream,
      output: NodeJS.WritableStream,
    ) => {
      try {
        const shouldContinue = output.write(chunk);
        if (!shouldContinue) {
          source.pause();
          output.once("drain", () => {
            source.resume();
          });
        }
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)));
      }
    };

    child.stdout.on("data", (chunk: Buffer) => {
      forwardOutput(chunk, child.stdout, options.stdout);
    });
    child.stderr.on("data", (chunk: Buffer) => {
      forwardOutput(chunk, child.stderr, options.stderr);
    });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      clearTimers();
      reject(new Error(`failed to spawn native runx '${options.command}': ${error.message}`));
    });
    child.on("close", (status, signal) => {
      clearTimers();
      if (settled) return;
      settled = true;
      if (timedOut) {
        reject(new Error(`native runx ${options.args.join(" ")} timed out after ${options.timeoutMs}ms.`));
        return;
      }
      resolve({ status, signal });
    });
  });
}

function nativeRunxError(args: readonly string[], result: NativeRunxProcessResult): Error {
  return new Error(
    `native runx ${args.join(" ")} failed with ${nativeRunxExitDescription(result)}: ${firstNonEmpty(result.stderr, result.stdout, "no output")}`,
  );
}

function nativeRunxTimeoutMs(options: NativeRunxOptions): number {
  return options.timeoutMs
    ?? parsePositiveInt(options.env.RUNX_RUST_CLI_TIMEOUT_MS)
    ?? DEFAULT_NATIVE_RUNX_TIMEOUT_MS;
}

function nativeRunxCwd(options: NativeRunxOptions): string {
  return options.cwd ?? options.env.RUNX_CWD ?? process.cwd();
}

function nativeRunxEnv(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  return {
    ...process.env,
    ...env,
    NO_COLOR: "1",
    RUNX_RUST_CLI: "1",
  };
}

function nativeRunxExitDescription(result: NativeRunxExitResult): string {
  if (result.status !== null) {
    return `exit ${result.status}`;
  }
  if (result.signal) {
    return `signal ${result.signal}`;
  }
  return "unknown status";
}

interface SupportedPlatformsManifest {
  readonly nativePackages?: Record<string, NativePackageTarget | undefined>;
}

interface NativePackageTarget {
  readonly package?: string;
  readonly binary?: string;
}

function resolveVerifiedPlatformNativeRunxBinary(): string | undefined {
  const platformKey = `${process.platform}-${process.arch}`;
  const cliPackageRoot = resolveCliPackageRoot();
  const target = readSupportedPlatforms(cliPackageRoot).nativePackages?.[platformKey];
  if (!target?.package || !target.binary) {
    return undefined;
  }

  let packageJsonPath: string;
  try {
    packageJsonPath = requireFromCli.resolve(`${target.package}/package.json`, { paths: [cliPackageRoot] });
  } catch {
    return undefined;
  }
  return verifyNativePackage(target.package, packageJsonPath, platformKey, target.binary);
}

function readSupportedPlatforms(cliPackageRoot: string): SupportedPlatformsManifest {
  const manifestPath = path.join(cliPackageRoot, "native", "supported-platforms.json");
  if (!existsSync(manifestPath)) {
    return {};
  }
  return readJson(manifestPath) as SupportedPlatformsManifest;
}

function verifyNativePackage(
  packageName: string,
  packageJsonPath: string,
  expectedPlatform: string,
  expectedBinary: string,
): string {
  const packageRoot = path.dirname(packageJsonPath);
  const binaryPath = path.join(packageRoot, expectedBinary);
  const manifest = readJson(packageJsonPath);
  if (readString(manifest, "name") !== packageName) {
    throw new Error(`runx native package mismatch: expected ${packageName}, found ${readString(manifest, "name") ?? "<missing>"}`);
  }
  if (!existsSync(binaryPath)) {
    throw new Error(`runx native binary is missing: ${binaryPath}`);
  }
  const binary = statSync(binaryPath);
  if (!binary.isFile() || (process.platform !== "win32" && (binary.mode & 0o111) === 0)) {
    throw new Error(`runx native binary is not executable: ${binaryPath}`);
  }

  const checksumPath = path.join(packageRoot, "native", "checksums.json");
  const checksum = readJson(checksumPath);
  if (readString(checksum, "platform") !== expectedPlatform) {
    throw new Error(`runx checksum platform mismatch: expected ${expectedPlatform}, found ${readString(checksum, "platform") ?? "<missing>"}`);
  }
  if (readString(checksum, "binary") !== expectedBinary) {
    throw new Error(`runx checksum binary mismatch: expected ${expectedBinary}, found ${readString(checksum, "binary") ?? "<missing>"}`);
  }
  const digest = createHash("sha256").update(readFileSync(binaryPath)).digest("hex");
  if (readString(checksum, "sha256") !== digest) {
    throw new Error("runx native binary checksum verification failed");
  }
  return binaryPath;
}

function resolveCliPackageRoot(): string {
  let directory = fileURLToPath(new URL(".", import.meta.url));
  while (true) {
    const packageJsonPath = path.join(directory, "package.json");
    if (existsSync(packageJsonPath)) {
      const manifest = readJson(packageJsonPath);
      if (readString(manifest, "name") === CLI_PACKAGE_NAME) {
        return directory;
      }
    }
    const parent = path.dirname(directory);
    if (parent === directory) {
      throw new Error(`could not locate ${CLI_PACKAGE_NAME} package root from ${fileURLToPath(import.meta.url)}`);
    }
    directory = parent;
  }
}

function readJson(filePath: string): unknown {
  try {
    return JSON.parse(readFileSync(filePath, "utf8")) as unknown;
  } catch (error) {
    throw new Error(`failed to read ${filePath}: ${errorMessage(error)}`);
  }
}

function readString(value: unknown, key: string): string | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : undefined;
}
