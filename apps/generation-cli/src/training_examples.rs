use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::Mutex,
};

use dataset_core::domain::SnapshotSplit;
use tempfile::NamedTempFile;
use training_core::domain::{TrainingExample, TrainingExampleSource};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
struct RecordLocation {
    offset: u64,
    length: usize,
}

pub struct TrainingExampleSpoolBuilder {
    file: NamedTempFile,
    training: Vec<RecordLocation>,
    training_member_ids: Vec<Uuid>,
    validation: Vec<RecordLocation>,
}

impl TrainingExampleSpoolBuilder {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            file: NamedTempFile::new().map_err(source_error)?,
            training: Vec::new(),
            training_member_ids: Vec::new(),
            validation: Vec::new(),
        })
    }

    pub fn push(&mut self, split: SnapshotSplit, example: &TrainingExample) -> Result<(), String> {
        if !matches!(split, SnapshotSplit::Train | SnapshotSplit::Validation) {
            return Ok(());
        }
        let bytes = serde_json::to_vec(example).map_err(source_error)?;
        let writer = self.file.as_file_mut();
        let location = RecordLocation {
            offset: writer.stream_position().map_err(source_error)?,
            length: bytes.len(),
        };
        writer.write_all(&bytes).map_err(source_error)?;
        match split {
            SnapshotSplit::Train => {
                self.training.push(location);
                self.training_member_ids.push(example.snapshot_member_id);
            }
            SnapshotSplit::Validation => self.validation.push(location),
            SnapshotSplit::Test => unreachable!("test rows return before serialization"),
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<TrainingExampleSpool, String> {
        self.file.as_file_mut().sync_all().map_err(source_error)?;
        let reader = self.file.reopen().map_err(source_error)?;
        Ok(TrainingExampleSpool {
            _file: self.file,
            reader: Mutex::new(reader),
            training: self.training,
            training_member_ids: self.training_member_ids,
            validation: self.validation,
        })
    }
}

pub struct TrainingExampleSpool {
    _file: NamedTempFile,
    reader: Mutex<File>,
    training: Vec<RecordLocation>,
    training_member_ids: Vec<Uuid>,
    validation: Vec<RecordLocation>,
}

impl TrainingExampleSpool {
    fn read(&self, locations: &[RecordLocation]) -> Result<Vec<TrainingExample>, String> {
        let mut reader = self
            .reader
            .lock()
            .map_err(|_| "training example spool lock is poisoned".to_owned())?;
        locations
            .iter()
            .map(|location| {
                reader
                    .seek(SeekFrom::Start(location.offset))
                    .map_err(source_error)?;
                let mut bytes = vec![0_u8; location.length];
                reader.read_exact(&mut bytes).map_err(source_error)?;
                serde_json::from_slice(&bytes).map_err(source_error)
            })
            .collect()
    }
}

impl TrainingExampleSource for TrainingExampleSpool {
    fn training_member_ids(&self) -> &[Uuid] {
        &self.training_member_ids
    }

    fn validation_len(&self) -> usize {
        self.validation.len()
    }

    fn training_batch(&self, indices: &[usize]) -> Result<Vec<TrainingExample>, String> {
        let locations = indices
            .iter()
            .map(|index| {
                self.training
                    .get(*index)
                    .copied()
                    .ok_or_else(|| format!("training example index is out of range: {index}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.read(&locations)
    }

    fn validation_batch(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<TrainingExample>, String> {
        if offset > self.validation.len() {
            return Err(format!(
                "validation example offset is out of range: {offset}"
            ));
        }
        self.read(&self.validation[offset..self.validation.len().min(offset.saturating_add(limit))])
    }
}

fn source_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use training_core::domain::{TrainingExample, TrainingExampleSource};
    use uuid::Uuid;

    use super::TrainingExampleSpoolBuilder;

    #[test]
    fn reads_selected_training_rows_and_bounded_validation_pages() {
        let mut builder = TrainingExampleSpoolBuilder::new().expect("builder");
        for (index, split) in [
            dataset_core::domain::SnapshotSplit::Train,
            dataset_core::domain::SnapshotSplit::Validation,
            dataset_core::domain::SnapshotSplit::Train,
            dataset_core::domain::SnapshotSplit::Test,
        ]
        .into_iter()
        .enumerate()
        {
            builder
                .push(
                    split,
                    &TrainingExample {
                        snapshot_member_id: Uuid::from_u128(index as u128 + 1),
                        text: format!("row {index}"),
                        label: "label".into(),
                    },
                )
                .expect("append");
        }
        let source = builder.finish().expect("source");

        let training = source.training_batch(&[1, 0]).expect("training batch");
        assert_eq!(training[0].text, "row 2");
        assert_eq!(training[1].text, "row 0");
        assert_eq!(source.validation_len(), 1);
        assert_eq!(
            source.validation_batch(0, 1).expect("validation")[0].text,
            "row 1"
        );
    }
}
