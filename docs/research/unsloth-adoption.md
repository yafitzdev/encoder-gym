# Run-loop usability improvements

Independent implementations informed by the [source review](unsloth-review-2026-09-19.md).
No Unsloth Studio source is copied. Evaluation redesign and replacing the
training backend are outside this change.

Each feature is verified and committed independently:

1. Launch summary and actionable readiness: exact immutable inputs, finite
   settings, local credential availability, and historical authority.
2. Training telemetry: safe numerical observations, persisted per iteration,
   unavailable values remain unavailable, checkpoint saving is not completion.
3. Dataset repair-plan projection: evidence and requested allocation versus
   actual accepted changes, read from persisted facts through CLI and desktop.
4. Bounded generation canary: preview and qualify a small sample before the
   remaining generation, retain receipts, preserve finite authorization and
   replay safety. Historical executions keep their original policy.
5. Development-case comparison: saved baseline and exact candidate predictions,
   both improvements and regressions, no rerunning inference or sealed access.

Verification uses deterministic tests and isolated fixtures, not paid provider
calls, production datasets, real training, final evaluation, or promotion.
