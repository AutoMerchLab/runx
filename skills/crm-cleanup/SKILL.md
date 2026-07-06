---
name: crm-cleanup
description: "Keeps pipeline data from rotting after calls. Reads transcript and CRM schema, extracts takeaways, maps them to allowed CRM fields, and emits a gated write proposal."
---
# CRM Cleanup

CRM Cleanup keeps pipeline data from rotting after calls. It reads an interaction transcript and `crm_schema`, extracts grounded takeaways, maps them to allowed CRM fields, and emits a gated `write_proposal`. It performs no live CRM write.
