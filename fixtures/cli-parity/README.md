# CLI Feature Parity Matrix

This directory captures the canonical native Rust CLI/runtime surface. The
matrix projects command syntax directly from `runx --help --json`.
`scripts/generate-cli-feature-parity.ts` adds only test, effect, and runtime
surface annotations keyed by native command name.

Required exit-code coverage: `"exitCodes": [0, 1, 2, 3, 64]`.

## Files

- `commands.json`: native usage/options plus exit-code, output, receipt, and
  side-effect coverage.
- `runtime-surfaces.json`: non-help runtime surfaces that must not disappear
  during a Rust rebuild.
- `cases/oracle.json`: executable or validation-only oracle cases.

## Parity Rules

- JSON output and receipt behavior are schema-exact.
- Human output is semantic and may be normalized for timestamps, paths,
  receipt ids, and platform-specific wording.
- Live providers are replaced by deterministic mocks, fixtures, or local
  protocol servers.
- Native CLI candidates must pass this matrix before packaging.
