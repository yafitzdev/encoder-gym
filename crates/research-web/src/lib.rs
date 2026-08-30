//! Replaceable web-search, page-fetch, and offline-corpus research adapters.

use std::{collections::BTreeSet, net::IpAddr, time::Duration};

use research_core::{
    brief::SourcePolicy,
    evidence::{FetchRequest, SearchRequest, SearchResult, UntrustedPage},
    ports::{BoxFuture, PageFetcher, ResearchAdapterError, SearchProvider},
};
use serde::{Deserialize, Serialize};

const BRAVE_WEB_SEARCH_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusDocument {
    pub url: String,
    pub title: String,
    pub source_class: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub content: String,
    #[serde(default = "plain_text")]
    pub media_type: String,
}

fn plain_text() -> String {
    "text/plain".into()
}

#[derive(Debug, Clone)]
pub struct CorpusWebAdapter {
    documents: Vec<CorpusDocument>,
}

impl CorpusWebAdapter {
    pub fn from_json(bytes: &[u8]) -> Result<Self, ResearchAdapterError> {
        let documents: Vec<CorpusDocument> =
            serde_json::from_slice(bytes).map_err(adapter_error)?;
        if documents.is_empty() {
            return Err(ResearchAdapterError(
                "offline research corpus must contain at least one document".into(),
            ));
        }
        let mut urls = BTreeSet::new();
        for document in &documents {
            if !urls.insert(document.url.as_str()) {
                return Err(ResearchAdapterError(format!(
                    "offline research corpus repeats URL {:?}",
                    document.url
                )));
            }
            if document.title.trim().is_empty()
                || document.source_class.trim().is_empty()
                || document.content.trim().is_empty()
            {
                return Err(ResearchAdapterError(
                    "offline corpus title, source_class, and content must not be empty".into(),
                ));
            }
        }
        Ok(Self { documents })
    }
}

impl SearchProvider for CorpusWebAdapter {
    fn search(
        &self,
        request: SearchRequest,
    ) -> BoxFuture<'_, Result<Vec<SearchResult>, ResearchAdapterError>> {
        Box::pin(async move {
            let terms = search_terms(&request.query);
            let requested_classes = request
                .source_classes
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect::<BTreeSet<_>>();
            let maximum = usize::try_from(request.maximum_results).map_err(adapter_error)?;
            Ok(self
                .documents
                .iter()
                .filter(|document| {
                    (requested_classes.is_empty()
                        || requested_classes.contains(&document.source_class.to_ascii_lowercase()))
                        && terms.iter().all(|term| document_matches(document, term))
                })
                .take(maximum)
                .map(|document| SearchResult {
                    url: document.url.clone(),
                    title: document.title.clone(),
                    summary: bounded_summary(&document.content),
                    source_class: Some(document.source_class.clone()),
                })
                .collect())
        })
    }
}

impl PageFetcher for CorpusWebAdapter {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> BoxFuture<'_, Result<UntrustedPage, ResearchAdapterError>> {
        Box::pin(async move {
            let document = self
                .documents
                .iter()
                .find(|document| document.url == request.url)
                .ok_or_else(|| ResearchAdapterError("offline corpus URL was not found".into()))?;
            if document.content.len() as u64 > request.maximum_bytes {
                return Err(ResearchAdapterError(format!(
                    "offline page exceeds the requested {} byte limit",
                    request.maximum_bytes
                )));
            }
            // Corpus URLs are validated by the host's persisted source policy.
            UntrustedPage::create(
                &SourcePolicy::default(),
                document.url.clone(),
                document.title.clone(),
                Some(document.media_type.clone()),
                document.content.clone(),
            )
            .map_err(adapter_error)
        })
    }
}

#[derive(Debug, Clone)]
pub struct BraveSearchProvider {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
}

impl BraveSearchProvider {
    pub fn new(api_key: String) -> Result<Self, ResearchAdapterError> {
        if api_key.trim().is_empty() {
            return Err(ResearchAdapterError(
                "Brave Search API key must not be empty".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(adapter_error)?;
        Ok(Self {
            client,
            api_key,
            endpoint: BRAVE_WEB_SEARCH_ENDPOINT.into(),
        })
    }
}

impl SearchProvider for BraveSearchProvider {
    fn search(
        &self,
        request: SearchRequest,
    ) -> BoxFuture<'_, Result<Vec<SearchResult>, ResearchAdapterError>> {
        Box::pin(async move {
            let count = request.maximum_results.to_string();
            let response = self
                .client
                .get(&self.endpoint)
                .header("Accept", "application/json")
                .header("X-Subscription-Token", &self.api_key)
                .query(&[("q", request.query.as_str()), ("count", count.as_str())])
                .send()
                .await
                .map_err(adapter_error)?;
            if !response.status().is_success() {
                return Err(ResearchAdapterError(format!(
                    "Brave Search returned HTTP {}",
                    response.status()
                )));
            }
            let payload: BraveResponse = response.json().await.map_err(adapter_error)?;
            Ok(payload
                .web
                .map_or_else(Vec::new, |web| web.results)
                .into_iter()
                .take(request.maximum_results as usize)
                .map(|result| SearchResult {
                    url: result.url,
                    title: result.title,
                    summary: result.description,
                    source_class: None,
                })
                .collect())
        })
    }
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: Option<BraveWeb>,
}

#[derive(Debug, Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Clone)]
pub struct HttpPageFetcher {
    client: reqwest::Client,
    source_policy: SourcePolicy,
}

impl HttpPageFetcher {
    pub fn new(source_policy: SourcePolicy) -> Result<Self, ResearchAdapterError> {
        let redirect_policy = source_policy.clone();
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent("encoder-gym-authenticity-research/0.1")
            .redirect(reqwest::redirect::Policy::custom(
                move |attempt| match safe_permitted_url(&redirect_policy, attempt.url().as_str()) {
                    Ok(true) => attempt.follow(),
                    _ => attempt.stop(),
                },
            ))
            .build()
            .map_err(adapter_error)?;
        Ok(Self {
            client,
            source_policy,
        })
    }
}

impl PageFetcher for HttpPageFetcher {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> BoxFuture<'_, Result<UntrustedPage, ResearchAdapterError>> {
        Box::pin(async move {
            if !safe_permitted_url(&self.source_policy, &request.url)? {
                return Err(ResearchAdapterError(
                    "source policy rejected page URL".into(),
                ));
            }
            let mut response = self
                .client
                .get(&request.url)
                .send()
                .await
                .map_err(adapter_error)?;
            if !response.status().is_success() {
                return Err(ResearchAdapterError(format!(
                    "page fetch returned HTTP {}",
                    response.status()
                )));
            }
            if !safe_permitted_url(&self.source_policy, response.url().as_str())? {
                return Err(ResearchAdapterError(
                    "source policy rejected final page URL".into(),
                ));
            }
            if response
                .content_length()
                .is_some_and(|length| length > request.maximum_bytes)
            {
                return Err(ResearchAdapterError(
                    "page Content-Length exceeds the requested byte limit".into(),
                ));
            }
            let media_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(adapter_error)? {
                let next = bytes.len().saturating_add(chunk.len());
                if next as u64 > request.maximum_bytes {
                    return Err(ResearchAdapterError(
                        "page body exceeds the requested byte limit".into(),
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let content = String::from_utf8_lossy(&bytes).into_owned();
            let title = response
                .url()
                .host_str()
                .unwrap_or("retrieved page")
                .to_owned();
            UntrustedPage::create(
                &self.source_policy,
                response.url().to_string(),
                title,
                media_type,
                content,
            )
            .map_err(adapter_error)
        })
    }
}

fn safe_permitted_url(policy: &SourcePolicy, value: &str) -> Result<bool, ResearchAdapterError> {
    let url = url::Url::parse(value).map_err(adapter_error)?;
    let host = url
        .host_str()
        .ok_or_else(|| ResearchAdapterError("source URL has no host".into()))?;
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| !is_global_ip(address))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Ok(false);
    }
    policy.permits_url(value).map_err(adapter_error)
}

fn is_global_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(value) => {
            !(value.is_private()
                || value.is_loopback()
                || value.is_link_local()
                || value.is_broadcast()
                || value.is_documentation()
                || value.is_unspecified())
        }
        IpAddr::V6(value) => {
            !(value.is_loopback() || value.is_unspecified() || value.is_unique_local())
        }
    }
}

fn search_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn document_matches(document: &CorpusDocument, term: &str) -> bool {
    document.title.to_ascii_lowercase().contains(term)
        || document.content.to_ascii_lowercase().contains(term)
        || document
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(term))
}

fn bounded_summary(content: &str) -> String {
    content.chars().take(240).collect()
}

fn adapter_error(error: impl std::fmt::Display) -> ResearchAdapterError {
    ResearchAdapterError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> CorpusWebAdapter {
        CorpusWebAdapter::from_json(
            br#"[{"url":"https://example.com/chat","title":"Support chat examples","source_class":"documentation","tags":["authentic billing"],"content":"Messages are short and omit greetings."}]"#,
        )
        .expect("corpus")
    }

    #[tokio::test]
    async fn corpus_search_and_fetch_are_deterministic_and_bounded() {
        let adapter = corpus();
        let results = adapter
            .search(SearchRequest {
                query: "authentic billing".into(),
                source_classes: vec!["documentation".into()],
                maximum_results: 3,
            })
            .await
            .expect("search");
        assert_eq!(results.len(), 1);
        let page = adapter
            .fetch(FetchRequest {
                url: results[0].url.clone(),
                maximum_bytes: 1_000,
            })
            .await
            .expect("fetch");
        assert!(
            page.delimited_for_agent()
                .contains("untrusted_research_source")
        );
        assert!(
            adapter
                .fetch(FetchRequest {
                    url: results[0].url.clone(),
                    maximum_bytes: 5,
                })
                .await
                .is_err()
        );
    }

    #[test]
    fn rejects_local_and_credential_bearing_urls() {
        let policy = SourcePolicy::default();
        assert!(!safe_permitted_url(&policy, "http://127.0.0.1/a").unwrap());
        assert!(!safe_permitted_url(&policy, "https://user@example.com/a").unwrap());
    }
}
