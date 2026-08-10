export function checkSchemaChange(inputs) {
  const policy = isRecord(inputs.compatibility_policy) ? inputs.compatibility_policy : {};
  const findings = validateRequest(inputs, policy);
  if (findings.length > 0) {
    return {
      schema_check: {
        schema: "runx.schema_check.v1",
        decision: "refused",
        policy_result: "invalid_input",
        breaking_changes: [],
        validation_results: [],
        migration_notes: [],
        proposal: null,
        validation: { status: "fail", findings },
      },
    };
  }

  const samples = inputs.sample_payloads;
  const { proposed, breaking, notes } = compareSchemas(inputs.current_schema, inputs.proposed_schema, policy);
  const validationResults = samples.map((sample, index) => {
    const errors = validateSample(sample, inputs.proposed_schema);
    return { sample_index: index, valid: errors.length === 0, errors };
  });
  const samplesValid = validationResults.every((result) => result.valid);
  const policyAllows = policy.breaking_allowed === true || breaking.length === 0;
  const compatible = policyAllows && samplesValid;
  const migrationNotes = [
    ...notes,
    ...coverageNotes(samples, proposed),
    ...(samplesValid
      ? []
      : [{ path: "$", kind: "sample_validation_failed", note: "One or more samples failed proposed schema validation." }]),
  ];
  return {
    schema_check: {
      schema: "runx.schema_check.v1",
      decision: compatible ? "compatible" : "refused",
      policy_result: policyAllows ? "allowed" : "blocked_by_policy",
      breaking_changes: breaking,
      validation_results: validationResults,
      migration_notes: migrationNotes,
      proposal: compatible
        ? {
            status: "proposed",
            gate: "schema-publisher-or-human-approver",
            live_write_performed: false,
            current_schema_digest: requiredDigest(inputs.current_digest),
            proposed_schema_digest: requiredDigest(inputs.proposed_digest),
            samples_digest: requiredDigest(inputs.samples_digest),
            changed_paths: [...new Set(notes.map((note) => note.path))].sort(),
          }
        : null,
      validation: { status: "pass", findings: [] },
    },
  };
}

function validateRequest(inputs, policy) {
  const findings = [];
  if (!isRecord(inputs.current_schema)) {
    findings.push({ code: "schema.invalid", message: "current_schema must be a JSON schema object." });
  }
  if (!isRecord(inputs.proposed_schema)) {
    findings.push({ code: "schema.invalid", message: "proposed_schema must be a JSON schema object." });
  }
  if (!Array.isArray(inputs.sample_payloads) || inputs.sample_payloads.length === 0) {
    findings.push({ code: "samples.invalid", message: "sample_payloads must be a non-empty array of payloads." });
  }
  if (!isRecord(inputs.compatibility_policy)) {
    findings.push({ code: "policy.invalid", message: "compatibility_policy must be an object." });
  } else if (policy.breaking_allowed !== undefined && typeof policy.breaking_allowed !== "boolean") {
    findings.push({ code: "policy.invalid", message: "compatibility_policy.breaking_allowed must be a boolean." });
  }
  return findings;
}

function requiredDigest(value) {
  if (typeof value !== "string" || !value.startsWith("sha256:")) {
    throw new Error("native digest evidence is missing");
  }
  return value;
}

function compareSchemas(currentSchema, proposedSchema, policy) {
  const current = normalizeSchema(currentSchema);
  const proposed = normalizeSchema(proposedSchema);
  const breaking = [];
  const notes = [];
  const paths = new Set([...current.keys(), ...proposed.keys()]);

  for (const path of [...paths].sort()) {
    const oldContract = current.get(path);
    const newContract = proposed.get(path);
    if (oldContract && !newContract) {
      breaking.push(change(path, oldContract, newContract, "field_removed"));
      continue;
    }
    if (!oldContract && newContract) {
      notes.push({
        path,
        kind: newContract.required ? "new_required_field" : "new_optional_field",
        note: newContract.required
          ? "New required field can break existing callers."
          : "Additive optional field.",
      });
      if (newContract.required) {
        breaking.push(change(path, oldContract, newContract, "new_required_field"));
      }
      continue;
    }
    if (oldContract.type !== newContract.type) {
      breaking.push(change(path, oldContract, newContract, "type_changed"));
    }
    if (!oldContract.required && newContract.required) {
      breaking.push(change(path, oldContract, newContract, "field_became_required"));
    }
    if (enumNarrowed(oldContract.enum, newContract.enum)) {
      breaking.push(change(path, oldContract, newContract, "enum_narrowed"));
    } else if (enumExpanded(oldContract.enum, newContract.enum)) {
      notes.push({ path, kind: "enum_expanded", note: "Enum was expanded without removing existing values." });
    }
  }

  for (const requiredPath of policyPaths(policy.required_fields)) {
    if (!proposed.has(requiredPath)) {
      breaking.push({
        path: requiredPath,
        old_contract: describeContract(current.get(requiredPath)),
        new_contract: "absent",
        policy_rule: "policy_required_field_missing",
      });
    }
  }

  return { proposed, breaking, notes };
}

function normalizeSchema(schema, basePath = "$") {
  if (!isRecord(schema)) return new Map();
  const fields = new Map();
  const properties = isRecord(schema.properties) ? schema.properties : {};
  const required = new Set(Array.isArray(schema.required) ? schema.required : []);
  for (const [name, child] of Object.entries(properties)) {
    const path = basePath === "$" ? `$.${name}` : `${basePath}.${name}`;
    fields.set(path, contractFor(child, required.has(name)));
    if (isRecord(child) && child.type === "object") {
      for (const [nestedPath, nestedContract] of normalizeSchema(child, path).entries()) {
        fields.set(nestedPath, nestedContract);
      }
    }
  }
  return fields;
}

function contractFor(node, required) {
  const schema = isRecord(node) ? node : {};
  return {
    type: Array.isArray(schema.type) ? schema.type.join("|") : schema.type || "any",
    required,
    enum: Array.isArray(schema.enum) ? [...schema.enum] : null,
    additional_properties: schema.additionalProperties === undefined ? null : schema.additionalProperties,
  };
}

function describeContract(contract) {
  if (!contract) return "absent";
  const parts = [`type=${contract.type}`, `required=${contract.required}`];
  if (contract.enum) parts.push(`enum=[${contract.enum.map(String).join(",")}]`);
  if (contract.additional_properties !== null) parts.push(`additionalProperties=${contract.additional_properties}`);
  return parts.join("; ");
}

function change(path, oldContract, newContract, policyRule) {
  return {
    path,
    old_contract: describeContract(oldContract),
    new_contract: describeContract(newContract),
    policy_rule: policyRule,
  };
}

function policyPaths(value) {
  if (!Array.isArray(value)) return [];
  return value
    .map((path) => String(path || "").trim())
    .filter(Boolean)
    .map((path) => (path.startsWith("$.") ? path : `$.${path.replace(/^\./u, "")}`));
}

function enumNarrowed(oldEnum, newEnum) {
  if (!oldEnum || !newEnum) return false;
  return oldEnum.some((value) => !newEnum.includes(value));
}

function enumExpanded(oldEnum, newEnum) {
  if (!oldEnum || !newEnum) return false;
  return newEnum.some((value) => !oldEnum.includes(value)) && oldEnum.every((value) => newEnum.includes(value));
}

function validateSample(sample, schema, pointer = "$") {
  const errors = [];
  const expectedType = isRecord(schema) ? schema.type : undefined;
  if (expectedType) {
    const expected = Array.isArray(expectedType) ? expectedType : [expectedType];
    const actual = typeOfValue(sample);
    if (!expected.includes(actual)) {
      return [`${pointer} expected ${expected.join("|")}, got ${actual}`];
    }
  }
  if (isRecord(schema) && Array.isArray(schema.enum) && !schema.enum.includes(sample)) {
    errors.push(`${pointer} expected one of ${schema.enum.map(String).join(", ")}`);
  }
  if (isRecord(schema) && schema.type === "object") {
    const properties = isRecord(schema.properties) ? schema.properties : {};
    for (const field of Array.isArray(schema.required) ? schema.required : []) {
      if (!isRecord(sample) || !(field in sample)) {
        errors.push(`${pointer}.${field} is required`);
      }
    }
    if (isRecord(sample)) {
      for (const [field, value] of Object.entries(sample)) {
        if (properties[field]) {
          errors.push(...validateSample(value, properties[field], `${pointer}.${field}`));
        } else if (schema.additionalProperties === false) {
          errors.push(`${pointer}.${field} is not allowed`);
        }
      }
    }
  }
  return errors;
}

function typeOfValue(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  if (Number.isInteger(value)) return "integer";
  return typeof value;
}

function coverageNotes(samples, proposedFields) {
  const covered = new Set();
  for (const sample of samples) {
    for (const path of coveredPaths(sample)) covered.add(path);
  }
  return [...proposedFields.keys()].sort().map((path) => ({
    path,
    kind: covered.has(path) ? "sample_covered" : "sample_not_covered",
    note: covered.has(path)
      ? "At least one supplied sample includes this path."
      : "No supplied sample covers this path; coverage was not invented.",
  }));
}

function coveredPaths(sample, basePath = "$") {
  if (!isRecord(sample)) return new Set();
  const paths = new Set();
  for (const [field, value] of Object.entries(sample)) {
    const path = `${basePath}.${field}`;
    paths.add(path);
    for (const nested of coveredPaths(value, path)) paths.add(nested);
  }
  return paths;
}

function isRecord(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
