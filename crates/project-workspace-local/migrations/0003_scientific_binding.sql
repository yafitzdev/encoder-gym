CREATE TABLE scientific_bindings (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    baseline_revision_id TEXT NOT NULL,
    previous_binding_id TEXT,
    specification_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    binding_json TEXT NOT NULL,
    FOREIGN KEY (baseline_revision_id) REFERENCES baseline_revisions(id),
    FOREIGN KEY (previous_binding_id) REFERENCES scientific_bindings(id)
);

CREATE TABLE scientific_binding_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    project_id TEXT NOT NULL,
    active_binding_id TEXT NOT NULL,
    FOREIGN KEY (active_binding_id) REFERENCES scientific_bindings(id)
);
