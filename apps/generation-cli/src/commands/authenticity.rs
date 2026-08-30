use anyhow::Context;
use generation_core::validation::SourceExcerptNoveltyValidator;
use research_core::{ports::ResearchStore, profile::ResolvedAuthenticityContext};
use synthetic_data_sqlite::SqliteStore;

/// Reconstructs the validation-only source corpus pinned indirectly by an
/// immutable authenticity profile. Evidence excerpts never enter prompts.
pub async fn source_novelty_guard(
    store: &SqliteStore,
    context: Option<&ResolvedAuthenticityContext>,
) -> anyhow::Result<Option<SourceExcerptNoveltyValidator>> {
    let Some(context) = context else {
        return Ok(None);
    };
    let profile = store
        .get_profile(context.profile_id)
        .await?
        .with_context(|| format!("authenticity profile {} is missing", context.profile_id))?;
    anyhow::ensure!(
        profile.fingerprint == context.profile_fingerprint
            && profile.reproduce_fingerprint()? == profile.fingerprint,
        "authenticity profile does not reproduce the pinned generation context"
    );
    anyhow::ensure!(
        profile.evidence_ids.len() == profile.evidence_fingerprints.len(),
        "authenticity profile evidence identity is inconsistent"
    );
    let evidence = store.list_evidence(profile.run_id).await?;
    let by_id = evidence
        .iter()
        .map(|item| (item.id, item))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut excerpts = Vec::with_capacity(profile.evidence_ids.len());
    for (id, expected_fingerprint) in profile
        .evidence_ids
        .iter()
        .zip(profile.evidence_fingerprints.iter())
    {
        let item = by_id
            .get(id)
            .with_context(|| format!("profile evidence {id} is missing"))?;
        anyhow::ensure!(
            item.fingerprint == *expected_fingerprint
                && item.reproduce_fingerprint()? == item.fingerprint,
            "profile evidence {id} failed its fingerprint check"
        );
        excerpts.push(item.excerpt.as_str());
    }
    anyhow::ensure!(
        !excerpts.is_empty(),
        "approved authenticity profile has no evidence for novelty validation"
    );
    Ok(Some(SourceExcerptNoveltyValidator::new(excerpts)?))
}
