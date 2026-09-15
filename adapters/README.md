# adapters/

Process adapters that keep optional external runtimes behind Encoder Gym's
typed Rust contracts.

| Directory | Purpose |
|---|---|
| [`research-agent-pi/`](research-agent-pi/) | Host-governed JSON Lines bridge for bounded Pi research and proposal turns |

Adapters do not own project authority or durable scientific decisions. Their
outputs are validated and persisted by the corresponding Rust host components.
