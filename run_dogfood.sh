#!/bin/bash
cd /home/test/frantic_bounty79
runx skill ./skills/crm-cleanup --json --input transcript="The client confirmed the budget is \$10k and next step is to send a proposal." --input crm_schema="{\"fields\": [\"budget\", \"next_step\"]}" > dogfood_receipt.json
