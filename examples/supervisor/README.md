# Offline generation-supervisor example

1. Create a dataset and generation plan.
2. Replace the all-zero `plan_id` in `contract.json` with that plan ID.
3. Build the Pi sidecar with `npm install` and `npm run build` in
   `adapters/research-agent-pi`.
4. Follow the offline command sequence in
   [`docs/generation-quality-supervisor.md`](../../docs/generation-quality-supervisor.md).

The example deliberately uses the repairable fake generator and deterministic
blind evaluator. Its first segment is shortcut-heavy; `diagnosis.json` proposes
a bounded guidance repair that makes the canary pass. It performs no network
or paid-provider work.
