# Contract Schema Consumer Inventory

Date: 2026-05-25

Scope: consumers of the committed `oss/schemas/*.schema.json` documents and
`@runxhq/contracts` schema exports during `rust-contract-pipeline-inversion`.

## Commands

- `rg -n "oss/schemas|schemas/.*schema\\.json|runxContractSchemas|validateContract|@runxhq/contracts|Value\\.Check|TypeBox|@sinclair/typebox|contracts/src/schemas" . ../cloud`
- `rg -n "schema_hash|schemaHash|sha256.*schema|hash.*schema|\\$id|x-runx-schema|properties\\]|\\.properties|required\\]|\\.required|patternProperties|additionalProperties|JSONSchema|JsonSchema|schemas/.*\\.schema\\.json|readFileSync\\(.*schema" . ../cloud`

## Inventory

| Consumer | Use | Structural schema dependency | Cutover result |
| --- | --- | --- | --- |
| `@runxhq/contracts` | Public TS validators, schema exports, OpenAPI builders | Yes, but in-package | Now sources published contract and auxiliary schema artifacts from `packages/contracts/src/schema-artifacts.ts`, generated from Rust-emitted `oss/schemas/*.schema.json`; validation resolves schemas by `$id` to the Rust-generated artifact before Ajv compilation. |
| OSS packages (`adapters`, `core`, `runtime-local`, `host-adapters`, `cli`) | Import validators/types from `@runxhq/contracts` | No direct schema-document hashing or structure pin found | Continue through `@runxhq/contracts`; validation goes through Ajv over generated artifacts. |
| Cloud packages (`cloud/packages/api`, `worker`, `auth`, `receipts-store`, `agent-runner`) | Import validators/types/OpenAPI builders from `@runxhq/contracts` | No direct hash pin found | Continue through `@runxhq/contracts`; OpenAPI remains generated from the package schema exports. |
| Rust receipt conformance (`crates/runx-receipts/tests/conformance.rs`) | Includes `schemas/receipt.schema.json` to validate emitted receipt data | Validates document, does not pin document bytes | Remains green as the committed schema document is Rust-emitted. |
| Rust contract wire conformance (`crates/runx-contracts/tests/schema_wire_conformance.rs`) | Compares Rust-emitted value domain with committed schema documents | Intentional gate | Expanded to all committed schema artifacts. |
| Tool manifests (`tools/**/manifest.json`) | Declare the canonical source, input, artifact, scope, and governance contract | No stored schema hash; `runx tool build` derives report hashes from the validated source manifest | Not affected by JSON Schema document layout changes. |
| OpenAPI generation (`packages/contracts/src/openapi*.ts`, cloud API docs builder) | Reads `runxContractSchemas` / schema exports | Yes | Uses generated artifact schemas from `@runxhq/contracts`; no stale TypeBox source remains in this path. |

## Result

No consumer was found that pins the byte shape or hash of the published
`oss/schemas/*.schema.json` documents. Consumers either validate data against the
schema documents or import validators/types through `@runxhq/contracts`.

The only structural schema consumer is the `@runxhq/contracts` package itself,
which now treats the Rust-emitted schema artifacts as its runtime validation
and aggregate export surface (`runxContractSchemas`, `runxAuxiliarySchemas`).
The remaining TS schema builders in `packages/contracts/src/schemas` exist as
typed convenience surfaces for package exports; any schema carrying a committed
artifact `$id` is resolved to the Rust-generated artifact before runtime
validation.
