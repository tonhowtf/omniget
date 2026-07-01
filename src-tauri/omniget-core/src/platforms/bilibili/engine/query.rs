use super::super::api::{ApiClient, BilibiliError, Result};
use super::super::cdn;

const MIN_VALID_SIZE: u64 = 10 * 1024;

#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CdnPreferences {
    pub alt_hosts: Vec<String>,
    pub prefer_alternatives: bool,
}

pub async fn resolve_best_url(
    client: &ApiClient,
    primary: &str,
    backups: &[String],
) -> Result<ResolvedStream> {
    resolve_best_url_with_cdn(client, primary, backups, &CdnPreferences::default()).await
}

pub async fn resolve_best_url_with_cdn(
    client: &ApiClient,
    primary: &str,
    backups: &[String],
    cdn_prefs: &CdnPreferences,
) -> Result<ResolvedStream> {
    let chain = if cdn_prefs.alt_hosts.is_empty() {
        let mut filtered = cdn::filter_blocked(
            &std::iter::once(primary.to_string())
                .chain(backups.iter().cloned())
                .collect::<Vec<_>>(),
        );
        if filtered.is_empty() {
            filtered.push(primary.to_string());
        }
        filtered
    } else {
        cdn::expand_with_alternatives(
            primary,
            backups,
            &cdn_prefs.alt_hosts,
            cdn_prefs.prefer_alternatives,
        )
    };

    let mut last_err: Option<BilibiliError> = None;
    for url in &chain {
        match client.head_content_length(url).await {
            Ok(Some(size)) if size >= MIN_VALID_SIZE => {
                return Ok(ResolvedStream {
                    url: url.clone(),
                    size,
                });
            }
            Ok(Some(_)) => {
                last_err = Some(BilibiliError::ContentUnavailable);
            }
            Ok(None) => {
                return Ok(ResolvedStream {
                    url: url.clone(),
                    size: 0,
                });
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or(BilibiliError::ContentUnavailable))
}
