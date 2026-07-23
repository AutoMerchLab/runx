---
name: external-adapter-echo
description: External-adapter sub-skill; a governed subprocess adapter that echoes its inputs.
---
A minimal external adapter (`runx.external_adapter.v1`). The runtime resolves the
manifest, spawns the declared subprocess under the governed sandbox, hands it the
invocation over stdio, and seals the adapter's reported result. Run it as a step
in a graph (the external-adapter source is a graph-step front, not a top-level
runner).
