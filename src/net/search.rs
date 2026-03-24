use reqwest::Client;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    pub uuid: Uuid,
    pub screenname: String,
}

#[derive(Deserialize)]
struct SearchResponse {
    users: Vec<SearchResult>,
}

/// Search users on the server by screenname.
pub async fn search_users(
    http: &Client,
    server_url: &str,
    auth_token: &str,
    query: &str,
) -> anyhow::Result<Vec<SearchResult>> {
    let resp = http
        .get(format!("{}/api/users/search", server_url))
        .query(&[("q", query)])
        .header("Authorization", auth_token)
        .send()
        .await?
        .error_for_status()?
        .json::<SearchResponse>()
        .await?;
    Ok(resp.users)
}
