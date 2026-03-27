use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
struct CreateInviteRequest {
    chat_uuid: Uuid,
    ttl: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    use_limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InviteInfo {
    pub code: String,
    pub display_code: String,
    pub expires_at: String,
    pub uses_remaining: Option<u32>,
}

#[derive(Deserialize)]
struct RedeemResponse {
    chat_uuid: Uuid,
    #[serde(default)]
    topic: String,
}

#[derive(Deserialize)]
struct ListResponse {
    invites: Vec<InviteInfo>,
}

pub async fn create_invite(
    http: &Client,
    server_url: &str,
    auth_token: &str,
    chat_uuid: Uuid,
    ttl: &str,
    use_limit: Option<u32>,
) -> anyhow::Result<InviteInfo> {
    let resp = http
        .post(format!("{}/api/invites", server_url))
        .header("Authorization", format!("Bearer {}", auth_token))
        .json(&CreateInviteRequest {
            chat_uuid,
            ttl: ttl.to_string(),
            use_limit,
        })
        .send()
        .await?
        .error_for_status()?
        .json::<InviteInfo>()
        .await?;
    Ok(resp)
}

pub async fn redeem_invite(
    http: &Client,
    server_url: &str,
    auth_token: &str,
    code: &str,
) -> anyhow::Result<(Uuid, String)> {
    let resp = http
        .post(format!("{}/api/invites/redeem", server_url))
        .header("Authorization", format!("Bearer {}", auth_token))
        .json(&serde_json::json!({"code": code}))
        .send()
        .await?
        .error_for_status()?
        .json::<RedeemResponse>()
        .await?;
    Ok((resp.chat_uuid, resp.topic))
}

pub async fn revoke_invite(
    http: &Client,
    server_url: &str,
    auth_token: &str,
    code: &str,
) -> anyhow::Result<()> {
    http.delete(format!("{}/api/invites/{}", server_url, code))
        .header("Authorization", format!("Bearer {}", auth_token))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

pub async fn list_invites(
    http: &Client,
    server_url: &str,
    auth_token: &str,
    chat_uuid: Uuid,
) -> anyhow::Result<Vec<InviteInfo>> {
    let resp = http
        .get(format!("{}/api/invites", server_url))
        .query(&[("chat_uuid", chat_uuid.to_string())])
        .header("Authorization", format!("Bearer {}", auth_token))
        .send()
        .await?
        .error_for_status()?
        .json::<ListResponse>()
        .await?;
    Ok(resp.invites)
}
