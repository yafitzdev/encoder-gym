//! Provider-neutral domain and application logic for synthetic-data generation.
//!
//! This crate must remain independent of SQLite, HTTP clients, CLI parsing, and
//! graphical presentation.

pub mod construction;
pub mod coverage;
pub mod deduplication;
pub mod dimensions;
pub mod domain;
pub mod export;
pub mod jobs;
pub mod parsing;
pub mod planning;
pub mod ports;
pub mod prompting;
pub mod strategy;
pub mod structured;
pub mod validation;
