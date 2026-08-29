//! Streaming JSONL/CSV adapters and composable import validation.

mod readers;
mod validation;

pub use readers::{CsvRecordReader, JsonlRecordReader, MappedRecord};
pub use validation::{
    DimensionValidator, ImportProcessor, ImportValidator, LabelValidator, NonEmptyTextValidator,
    NormalizedDuplicateValidator, ValidationPipeline,
};
