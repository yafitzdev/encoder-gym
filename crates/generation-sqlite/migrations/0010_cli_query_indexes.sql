-- Filters and stable pagination used by the local CLI read models.
CREATE INDEX generation_jobs_dataset_state_created_idx
    ON generation_jobs(dataset_id, state, created_at, id);
CREATE INDEX generation_jobs_plan_state_created_idx
    ON generation_jobs(plan_id, state, created_at, id);
CREATE INDEX generated_rows_dataset_status_created_idx
    ON generated_rows(dataset_id, validation_status, created_at, id);
CREATE INDEX generated_rows_job_status_created_idx
    ON generated_rows(generation_job_id, validation_status, created_at, id);
CREATE INDEX dataset_imports_dataset_state_created_idx
    ON dataset_imports(dataset_id, state, created_at, id);
CREATE INDEX training_runs_state_created_idx
    ON training_runs(state, created_at, id);
CREATE INDEX evaluation_runs_state_created_idx
    ON evaluation_runs(state, created_at, id);
CREATE INDEX optimization_proposals_dataset_created_idx
    ON optimization_proposals(dataset_id, created_at, id);
