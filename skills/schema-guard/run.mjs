import fs from 'fs';

function checkCompatibility(current, proposed, breaking_allowed, validation_results, migration_notes, path = '') {
  let isCompatible = true;
  for (const key of Object.keys(current)) {
    const fullPath = path ? `${path}.${key}` : key;
    if (!(key in proposed)) {
      if (!breaking_allowed) {
        isCompatible = false;
        validation_results.push(`Breaking change: removed field '${fullPath}'.`);
      } else {
        migration_notes.push(`Field '${fullPath}' was removed.`);
      }
    } else {
      if (typeof current[key] === 'object' && current[key] !== null && typeof proposed[key] === 'object' && proposed[key] !== null) {
        if (!checkCompatibility(current[key], proposed[key], breaking_allowed, validation_results, migration_notes, fullPath)) {
          isCompatible = false;
        }
      } else if (current[key] !== proposed[key]) {
        if (!breaking_allowed) {
          isCompatible = false;
          validation_results.push(`Breaking change: type or value of '${fullPath}' changed from ${current[key]} to ${proposed[key]}.`);
        } else {
          migration_notes.push(`Field '${fullPath}' changed from ${current[key]} to ${proposed[key]}.`);
        }
      }
    }
  }
  for (const key of Object.keys(proposed)) {
    const fullPath = path ? `${path}.${key}` : key;
    if (!(key in current)) {
      migration_notes.push(`Field '${fullPath}' was added.`);
    }
  }
  return isCompatible;
}

try {
  let input = fs.readFileSync(0, 'utf-8');
  if (input.charCodeAt(0) === 0xFEFF) {
    input = input.slice(1);
  }
  if (!input.trim()) {
    throw new Error('Empty input');
  }
  const data = JSON.parse(input);

  const current_schema = data.current_schema || {};
  const proposed_schema = data.proposed_schema || {};
  const compatibility_policy = data.compatibility_policy || {};
  const breaking_allowed = compatibility_policy.breaking_allowed === true;

  const validation_results = [];
  const migration_notes = [];

  const isCompatible = checkCompatibility(current_schema, proposed_schema, breaking_allowed, validation_results, migration_notes);

  const output = {
    compatibility: isCompatible,
    validation_results,
    migration_notes
  };

  if (isCompatible) {
    output.publish_schema_proposal = true;
  }

  console.log(JSON.stringify(output, null, 2)); if (!isCompatible) process.exit(1);
} catch (error) {
  console.log(JSON.stringify({
    compatibility: false,
    validation_results: [error.message || 'Unknown error occurred'],
    migration_notes: []
  }, null, 2));
  process.exit(0);
}
