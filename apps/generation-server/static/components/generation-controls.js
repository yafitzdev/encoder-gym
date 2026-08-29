import { api } from "/assets/api.js";

export function setupGenerationControls({ getPlanId, onJob, onFinished }) {
  const start = document.querySelector("#start-generation");
  const stop = document.querySelector("#stop-generation");
  let currentJobId = null;
  let timer = null;

  async function poll() {
    if (!currentJobId) return;
    const job = await api(`/api/jobs/${currentJobId}`);
    await onJob(job);
    if (["completed", "failed", "cancelled"].includes(job.state)) {
      clearInterval(timer);
      timer = null;
      stop.disabled = true;
      start.disabled = false;
      await onFinished(job);
    }
  }

  start.addEventListener("click", async () => {
    const planId = getPlanId();
    if (!planId) throw new Error("Save a generation plan first.");
    const job = await api("/api/jobs", {
      method: "POST",
      body: JSON.stringify({
        plan_id: planId,
        backend: document.querySelector("#generation-backend").value,
        batch_size: Number(document.querySelector("#batch-size").value),
      }),
    });
    currentJobId = job.id;
    start.disabled = true;
    stop.disabled = false;
    await onJob(job);
    timer = setInterval(() => poll().catch(reportError), 500);
    await poll();
  });

  stop.addEventListener("click", async () => {
    if (!currentJobId) return;
    await api(`/api/jobs/${currentJobId}/cancel`, { method: "POST" });
    stop.disabled = true;
  });
}

function reportError(error) {
  document.querySelector("#global-error").textContent = error.message;
}
