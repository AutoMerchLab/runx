function parseInput() {
  const inputStr = process.env.RUNX_INPUTS_JSON;
  if (!inputStr) {
    return refuse("No input provided via RUNX_INPUTS_JSON");
  }
  try {
    return JSON.parse(inputStr);
  } catch (e) {
    return refuse("Invalid JSON input");
  }
}

function parseSchema(schemaRaw) {
  if (typeof schemaRaw === 'object' && schemaRaw !== null) {
    return schemaRaw;
  }
  try {
    return JSON.parse(schemaRaw);
  } catch (e) {
    return refuse("Invalid crm_schema JSON");
  }
}

function main() {
  const parsedInput = parseInput();
  const transcript = parsedInput.transcript || "";
  const crm_schema_raw = parsedInput.crm_schema || "{}";
  
  const crm_schema = parseSchema(crm_schema_raw);
  const allowedFields = crm_schema.fields || [];
  
  if (!Array.isArray(allowedFields)) {
    return refuse("crm_schema.fields must be an array");
  }

  if (allowedFields.length === 0) {
    return refuse("No fields defined in schema");
  }

  const takeaways = [];
  const field_updates = {};

  for (const field of allowedFields) {
    const fieldName = field.replace(/_/g, ' ');
    // Try matching "[field] is [value]" or "set [field] to [value]"
    const regex1 = new RegExp(`${fieldName} is \\$?([\\w.,]+|.*?)(?=\\.| and | but |,$|$)`, 'i');
    const regex2 = new RegExp(`set ${fieldName} to \\$?([\\w.,]+|.*?)(?=\\.| and | but |,$|$)`, 'i');
    
    const match = transcript.match(regex1) || transcript.match(regex2);
    if (match) {
        let value = match[1].trim();
        if (field === 'budget' && !value.startsWith('$') && transcript.includes(`$${value}`)) {
            value = '$' + value;
        } else if (field === 'budget' && !value.startsWith('$')) {
            // Optional: formatting for budget if needed
        }
        takeaways.push(`Identified ${fieldName}: ${value}`);
        field_updates[field] = value;
    }
  }

  const write_proposal = Object.keys(field_updates).length > 0;

  seal({
      takeaways,
      field_updates,
      write_proposal
  });
}

function seal(data) {
  console.log(JSON.stringify(data, null, 2));
  process.exit(0);
}

function refuse(reason) {
  console.error(reason);
  process.exit(1);
}

main();
