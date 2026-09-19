//! Native input/context identities for bounded dataset investigation.
//!
//! The fixed Python projection calls Nomos's own validator, query renderer and
//! candidate renderer. Rust validates the complete fingerprint-only result and
//! never attempts to reproduce trainer-visible input semantics.

use super::*;
use std::io::Write;

const PROGRAM: &str = include_str!("inspect_training_inventory.py");
const PROTOCOL: &str = "nomos-training-inventory-v1";
const MAX_RESULT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NomosNativeInventoryMember {
    pub member_id: String,
    pub native_context_fingerprint: String,
    pub native_model_input_fingerprint: String,
    pub label_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NomosNativeInventory {
    pub protocol: String,
    pub request_fingerprint: String,
    pub dataset_fingerprint: String,
    pub rows: u64,
    pub members: Vec<NomosNativeInventoryMember>,
    #[serde(default)]
    pub fingerprint: String,
}

impl NomosNativeInventory {
    pub(crate) fn reproduce(&self) -> Result<String, EncoderTaskAdapterError> {
        let mut value = serde_json::to_value(self).map_err(adapter_error)?;
        value
            .as_object_mut()
            .expect("inventory is an object")
            .remove("fingerprint");
        artifact_core::fingerprint(&value).map_err(adapter_error)
    }

    fn validate(
        &self,
        request: &Value,
        member_ids: &BTreeSet<String>,
    ) -> Result<(), EncoderTaskAdapterError> {
        self.validate_population(member_ids)?;
        if self.request_fingerprint != request["fingerprint"].as_str().unwrap_or_default()
            || self.dataset_fingerprint
                != request["datasetFingerprint"].as_str().unwrap_or_default()
        {
            return Err(adapter_error(
                "Native training inventory does not match its exact request",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_population(
        &self,
        member_ids: &BTreeSet<String>,
    ) -> Result<(), EncoderTaskAdapterError> {
        let projected_ids = self
            .members
            .iter()
            .map(|member| member.member_id.clone())
            .collect::<BTreeSet<_>>();
        if self.protocol != PROTOCOL
            || !managed_training::canonical_fingerprint(&self.request_fingerprint)
            || !managed_training::canonical_fingerprint(&self.dataset_fingerprint)
            || self.rows != member_ids.len() as u64
            || self.members.len() != member_ids.len()
            || &projected_ids != member_ids
            || self
                .members
                .iter()
                .any(|member| member.member_id.is_empty() || member.member_id.len() > 128)
            || self.members.iter().any(|member| {
                [
                    &member.native_context_fingerprint,
                    &member.native_model_input_fingerprint,
                    &member.label_fingerprint,
                ]
                .into_iter()
                .any(|value| !managed_training::canonical_fingerprint(value))
            })
            || self.reproduce()? != self.fingerprint
        {
            return Err(adapter_error(
                "Native training inventory does not match its exact inputs",
            ));
        }
        Ok(())
    }

    pub fn by_member(&self) -> BTreeMap<&str, &NomosNativeInventoryMember> {
        self.members
            .iter()
            .map(|member| (member.member_id.as_str(), member))
            .collect()
    }
}

impl NomosBackend {
    /// Inspect one already-verified project dataset using the same native input
    /// renderer used by complete-population qualification. No provider or model
    /// is invoked and raw rendered inputs never leave the local child process.
    pub async fn inspect_training_inventory(
        &self,
        run_id: Uuid,
        dataset_fingerprint: &str,
        rows: &BTreeMap<String, Value>,
    ) -> Result<NomosNativeInventory, EncoderTaskAdapterError> {
        if run_id.is_nil()
            || !managed_training::canonical_fingerprint(dataset_fingerprint)
            || rows.is_empty()
        {
            return Err(adapter_error(
                "Native training inventory request is invalid",
            ));
        }
        for row in rows.values() {
            managed_training::validate_native_training_row(row)?;
        }
        self.verify_no_remote()?;
        self.verify_clean_worktree()?;
        let mut sources = BTreeMap::new();
        for relative in [
            format!("{}/dense_router.py", self.native_package.module()),
            format!("{}/generic_contracts.py", self.native_package.module()),
        ] {
            let path = self.resolve_existing(&relative)?;
            sources.insert(relative, prefixed(&sha256_file(&path)?));
        }
        let parent = managed_training::ensure_output_directory(
            &self.root,
            &[
                "runs",
                "encoder-gym-project-runs",
                &run_id.to_string(),
                "inventory",
            ],
        )?;
        let scratch = parent.join(format!(".tmp-{}", Uuid::new_v4()));
        fs::create_dir(&scratch).map_err(adapter_error)?;
        let result = async {
            let members_path = scratch.join("members.jsonl");
            let mut members_file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&members_path)
                .map_err(adapter_error)?;
            let mut digest = Sha256::new();
            for (member_id, row) in rows {
                let mut line = serde_json::to_vec(&json!({
                    "memberId": member_id,
                    "row": row,
                }))
                .map_err(adapter_error)?;
                line.push(b'\n');
                digest.update(&line);
                members_file.write_all(&line).map_err(adapter_error)?;
            }
            members_file.sync_all().map_err(adapter_error)?;
            drop(members_file);
            let members_bytes = fs::metadata(&members_path).map_err(adapter_error)?.len();
            let mut request = json!({
                "protocol": PROTOCOL,
                "programFingerprint": prefixed(&format!("{:x}", Sha256::digest(PROGRAM.as_bytes()))),
                "datasetFingerprint": dataset_fingerprint,
                "rows": rows.len(),
                "membersKey": workspace_relative(&self.root, &members_path)?,
                "membersBytes": members_bytes,
                "membersFingerprint": prefixed(&format!("{:x}", digest.finalize())),
                "sources": sources,
                "nativePackage": self.native_package.module(),
            });
            request["fingerprint"] = artifact_core::fingerprint(&request)
                .map_err(adapter_error)?
                .into();
            let request_path = scratch.join("request.json");
            let mut request_file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&request_path)
                .map_err(adapter_error)?;
            request_file
                .write_all(&serde_json::to_vec(&request).map_err(adapter_error)?)
                .map_err(adapter_error)?;
            request_file.sync_all().map_err(adapter_error)?;
            drop(request_file);
            self.run_bounded(
                &[
                    "-B".into(),
                    "-c".into(),
                    PROGRAM.into(),
                    workspace_relative(&self.root, &request_path)?,
                ],
                120,
            )
            .await?;
            let result_path = scratch.join("result.json");
            if fs::metadata(&result_path).map_err(adapter_error)?.len() > MAX_RESULT_BYTES {
                return Err(adapter_error("Native training inventory output exceeds its bound"));
            }
            let mut inventory: NomosNativeInventory =
                serde_json::from_slice(&fs::read(result_path).map_err(adapter_error)?)
                    .map_err(adapter_error)?;
            inventory.fingerprint = inventory.reproduce()?;
            inventory.validate(&request, &rows.keys().cloned().collect())?;
            Ok(inventory)
        }
        .await;
        if scratch.exists() {
            remove_exact_scratch_directory(&scratch, &parent)?;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_requires_exact_population_and_native_fingerprints() {
        let member = NomosNativeInventoryMember {
            member_id: "row-a".into(),
            native_context_fingerprint: artifact_core::fingerprint(&"context").unwrap(),
            native_model_input_fingerprint: artifact_core::fingerprint(&"input").unwrap(),
            label_fingerprint: artifact_core::fingerprint(&"label").unwrap(),
        };
        let mut inventory = NomosNativeInventory {
            protocol: PROTOCOL.into(),
            request_fingerprint: artifact_core::fingerprint(&"request").unwrap(),
            dataset_fingerprint: artifact_core::fingerprint(&"dataset").unwrap(),
            rows: 1,
            members: vec![member],
            fingerprint: String::new(),
        };
        inventory.fingerprint = inventory.reproduce().unwrap();
        let request = json!({
            "fingerprint": inventory.request_fingerprint,
            "datasetFingerprint": inventory.dataset_fingerprint,
        });
        inventory
            .validate(&request, &BTreeSet::from(["row-a".into()]))
            .unwrap();
        inventory.members[0].native_context_fingerprint = "bad".into();
        assert!(
            inventory
                .validate(&request, &BTreeSet::from(["row-a".into()]))
                .is_err()
        );
    }
}
