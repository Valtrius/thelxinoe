//! A read-only client for an existing Jellyfin server, authenticated with an API key.

use crate::error::{ApiError, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;

/// Items per page; a page of played episodes stays well under the response limit.
const PAGE: usize = 500;
const RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
/// A library larger than this is not one person's watch history.
const ITEM_LIMIT: usize = 200_000;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Server {
    pub url: String,
    pub api_key: String,
    pub name: String,
    pub version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct System {
    server_name: Option<String>,
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Account {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub policy: Policy,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Policy {
    #[serde(default)]
    pub is_administrator: bool,
    #[serde(default)]
    pub is_disabled: bool,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Item {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub kind: String,
    pub production_year: Option<i64>,
    pub path: Option<String>,
    #[serde(default)]
    pub provider_ids: HashMap<String, Option<String>>,
    pub series_id: Option<String>,
    pub series_name: Option<String>,
    pub parent_index_number: Option<i64>,
    pub index_number: Option<i64>,
    pub index_number_end: Option<i64>,
    #[serde(default)]
    pub user_data: UserData,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct UserData {
    #[serde(default)]
    pub played: bool,
    #[serde(default)]
    pub playback_position_ticks: i64,
    #[serde(default)]
    pub is_favorite: bool,
    pub last_played_date: Option<String>,
}

impl Item {
    /// A provider ID by Jellyfin's provider name, whatever its letter case.
    pub fn provider(&self, name: &str) -> Option<String> {
        self.provider_ids
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
    }
    /// When this person last played the item, as Unix seconds.
    pub fn last_played(&self) -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(self.user_data.last_played_date.as_deref()?)
            .ok()
            .map(|at| at.timestamp())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Page {
    #[serde(default)]
    items: Vec<Item>,
    #[serde(default)]
    total_record_count: usize,
}

/// Which of a person's items a query returns.
#[derive(Clone, Copy)]
pub(super) enum Filter {
    Played,
    Resumable,
    Favorite,
}
impl Filter {
    fn query(self) -> (&'static str, &'static str) {
        match self {
            Self::Played => ("IsPlayed", "Movie,Episode"),
            Self::Resumable => ("IsResumable", "Movie,Episode"),
            Self::Favorite => ("IsFavorite", "Movie,Episode,Series"),
        }
    }
}

pub(super) struct Jellyfin {
    http: reqwest::Client,
    base: url::Url,
    key: String,
}

/// The address an administrator entered, as Jellyfin's base URL without a trailing slash.
pub(super) fn address(input: &str) -> Result<String> {
    let invalid = || ApiError::bad("Enter the Jellyfin address, for example http://jellyfin:8096");
    let url = url::Url::parse(input.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

impl Jellyfin {
    pub fn new(url: &str, key: &str) -> Result<Self> {
        let base = url::Url::parse(&format!("{}/", address(url)?))
            .map_err(|_| ApiError::bad("Enter a valid Jellyfin address"))?;
        let key = key.trim();
        if key.is_empty() || key.len() > 256 || !key.chars().all(|c| c.is_ascii_graphic()) {
            return Err(ApiError::bad("Enter a Jellyfin API key"));
        }
        Ok(Self {
            // Jellyfin usually runs beside Thelxinoe, so system proxies are not used, and a
            // redirect could carry the key to another host.
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(anyhow::Error::from)?,
            base,
            key: key.to_owned(),
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let mut url = self.base.join(path).map_err(anyhow::Error::from)?;
        url.query_pairs_mut().extend_pairs(query);
        let shown = self.base.as_str().trim_end_matches('/').to_owned();
        let mut response = self
            .http
            .get(url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!(
                    "MediaBrowser Client=\"Thelxinoe\", Device=\"Thelxinoe\", DeviceId=\"thelxinoe-import\", Version=\"{}\", Token=\"{}\"",
                    thelxinoe_core::VERSION,
                    self.key
                ),
            )
            // Without a JSON Accept header Jellyfin answers in camelCase.
            .header(
                reqwest::header::ACCEPT,
                "application/json; profile=\"PascalCase\"",
            )
            .send()
            .await
            .map_err(|_| {
                ApiError::conflict(format!(
                    "Jellyfin did not answer at {shown}. Check the address and that this server can reach it."
                ))
            })?;
        let status = response.status();
        if status.is_redirection() {
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("another address");
            return Err(ApiError::conflict(format!(
                "Jellyfin redirected to {target}. Enter that address instead."
            )));
        }
        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return Err(ApiError::conflict(
                "Jellyfin refused the API key. Create one under Dashboard → API Keys.",
            ));
        }
        if !status.is_success() {
            return Err(ApiError::conflict(format!(
                "Jellyfin answered with HTTP {}",
                status.as_u16()
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::conflict("Jellyfin stopped answering mid-response"))?
        {
            if body.len() + chunk.len() > RESPONSE_LIMIT {
                return Err(ApiError::conflict("Jellyfin sent more data than expected"));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::conflict("This address did not answer like a Jellyfin server"))
    }

    /// Checks the key reaches an administrator view of the server, and names it.
    pub async fn identify(&self) -> Result<(String, String)> {
        let system: System = self.get("System/Info", &[]).await?;
        // Only an API key or an administrator can list every account.
        self.accounts().await?;
        Ok((
            system.server_name.unwrap_or_else(|| "Jellyfin".into()),
            system.version.unwrap_or_default(),
        ))
    }

    pub async fn accounts(&self) -> Result<Vec<Account>> {
        self.get("Users", &[]).await
    }

    fn items_query(user: &str, filter: Filter) -> Vec<(&str, &str)> {
        let (filters, types) = filter.query();
        vec![
            ("userId", user),
            ("recursive", "true"),
            ("filters", filters),
            ("includeItemTypes", types),
            ("fields", "Path,ProviderIds"),
            ("enableImages", "false"),
            ("enableUserData", "true"),
            ("enableTotalRecordCount", "true"),
        ]
    }

    pub async fn count(&self, user: &str, filter: Filter) -> Result<usize> {
        let mut query = Self::items_query(user, filter);
        query.push(("limit", "1"));
        Ok(self.get::<Page>("Items", &query).await?.total_record_count)
    }

    pub async fn items(&self, user: &str, filter: Filter) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        loop {
            let start = items.len().to_string();
            let limit = PAGE.to_string();
            let mut query = Self::items_query(user, filter);
            query.extend([("startIndex", start.as_str()), ("limit", limit.as_str())]);
            let page: Page = self.get("Items", &query).await?;
            let done = page.items.is_empty();
            items.extend(page.items);
            if done || items.len() >= page.total_record_count {
                return Ok(items);
            }
            if items.len() > ITEM_LIMIT {
                return Err(ApiError::conflict(
                    "This Jellyfin account has more items than an import can read",
                ));
            }
        }
    }

    /// The series these episodes belong to, for their provider IDs and paths.
    pub async fn series(&self, user: &str, ids: &[String]) -> Result<Vec<Item>> {
        let mut series = Vec::new();
        for batch in ids.chunks(100) {
            let ids = batch.join(",");
            let page: Page = self
                .get(
                    "Items",
                    &[
                        ("userId", user),
                        ("ids", &ids),
                        ("fields", "Path,ProviderIds"),
                        ("enableImages", "false"),
                        ("enableUserData", "false"),
                    ],
                )
                .await?;
            series.extend(page.items);
        }
        Ok(series)
    }
}
