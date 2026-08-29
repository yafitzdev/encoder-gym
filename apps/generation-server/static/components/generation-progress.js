export function renderGenerationProgress(container, job) {
  if (!job) {
    container.replaceChildren();
    return;
  }
  const progress = document.createElement("progress");
  progress.max = Math.max(job.requested_rows, 1);
  progress.value = Math.min(job.accepted_rows, progress.max);
  const text = document.createElement("p");
  text.textContent = `${job.state}: ${job.accepted_rows} accepted, ${job.rejected_rows} rejected, ${job.failed_requests} failed requests, ${Math.max(job.requested_rows - job.accepted_rows, 0)} remaining`;
  container.replaceChildren(progress, text);
  if (job.error_message) {
    const error = document.createElement("p");
    error.className = "error";
    error.textContent = job.error_message;
    container.append(error);
  }
}
