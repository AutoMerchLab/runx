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

  const takeaways = [];
  const field_updates = {};

  const budgetMatch = transcript.match(/budget is ([^.]+)/i);
  if (budgetMatch && allowedFields.includes('budget')) {
      takeaways.push(`Identified budget: ${budgetMatch[1].trim()}`);
      field_updates['budget'] = budgetMatch[1].trim();
  }

  const statusMatch = transcript.match(/status is ([^.]+)/i);
  if (statusMatch && allowedFields.includes('status')) {
      takeaways.push(`Identified status: ${statusMatch[1].trim()}`);
      field_updates['status'] = statusMatch[1].trim();
  }
  
  const nextStepsMatch = transcript.match(/next step is ([^.]+)/i);
  if (nextStepsMatch && allowedFields.includes('next_step')) {
      takeaways.push(`Identified next step: ${nextStepsMatch[1].trim()}`);
      field_updates['next_step'] = nextStepsMatch[1].trim();
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
