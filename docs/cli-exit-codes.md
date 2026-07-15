# CLI Exit Codes

Runx uses a small exit-code surface so scripts can branch without parsing
human output.

## Exit Code 0: Sealed

The command sealed successfully. For `runx skill`, `runx harness`, and native
history reads, the requested work or read operation succeeded.

Common follow-up:

```bash
runx history <receipt-id> --json
```

## Exit Code 1: Failure

The command ran but failed, was denied by policy, hit an invalid operation, or
found invalid requested output.

Common fixes:

- Read the stderr message first; it should name the failing command or policy.
- Re-run with `--json` when the command supports it.
- For harness failures, inspect `assertionErrors` in the JSON output.
- For `runx skill owner/name@version`, unsigned manifests, unknown trust keys,
  digest mismatches, and profile digest mismatches fail here before execution.

## Exit Code 2: Needs Agent

The run needs input, approval, or an agent act before it
can continue. In production mode (`RUNX_PRODUCTION=1`), unresolved cognitive
work is treated as a terminal failure but keeps exit code 2 so automation
can distinguish it from ordinary command failure.

Common fixes:

```bash
runx resume <run-id> answers.json
```

For required input, pass the missing `--input` value or the corresponding
kebab-case CLI flag.

## Exit Code 3: Verification Failed

The verification command completed, but the receipt, receipt tree, signature,
or notary evidence did not verify. This is distinct from exit code 1: the
verification machinery ran successfully and produced a verdict.

Common fixes:

- Read the verification findings in stdout, or re-run with `--json`.
- Check the trusted key, receipt lineage, and body/signature digests.
- Use exit code 1 for I/O, parsing, or runtime failures before a verdict exists.

## Exit Code 64: Usage

The command shape is not supported. This usually means the first positional
argument is not a known command or the command is missing its required action.

Common fixes:

```bash
runx --help
runx skill <skill-ref>
runx harness <fixture.yaml>
```
