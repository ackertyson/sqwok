use anyhow::{bail, Result};
use base64::Engine;
use std::path::Path;

pub async fn run_registration(server_url: &str, identity_dir: &Path) -> Result<()> {
    // Check for resume state
    let pending_path = identity_dir.join("pending_request");
    if pending_path.exists() {
        let request_uuid = std::fs::read_to_string(&pending_path)?.trim().to_string();
        let preview_len = request_uuid.len().min(8);
        println!(
            "Resuming verification for request {}...",
            &request_uuid[..preview_len]
        );
        return resume_from_polling(server_url, identity_dir, &request_uuid).await;
    }

    let email: String = dialoguer::Input::new()
        .with_prompt("Enter your email address to register or recover your account")
        .validate_with(|s: &String| -> Result<(), &str> {
            if s.contains('@') && s.len() <= 254 {
                Ok(())
            } else {
                Err("Please enter a valid email address")
            }
        })
        .interact_text()?;

    let screenname: String = dialoguer::Input::new()
        .with_prompt("Screen name")
        .validate_with(|s: &String| -> Result<(), &str> {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Err("Screen name cannot be empty")
            } else if trimmed.len() > 30 {
                Err("Screen name must be 30 characters or fewer")
            } else if trimmed.chars().any(|c| c.is_control()) {
                Err("Screen name cannot contain control characters")
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    let (email, screenname) = (email, screenname.trim().to_string());

    let request_uuid = uuid::Uuid::new_v4().to_string();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/register", server_url))
        .json(&serde_json::json!({
            "email": email,
            "screenname": screenname,
            "request_uuid": request_uuid
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        bail!("Registration failed: {}", resp.text().await?);
    }

    std::fs::create_dir_all(identity_dir)?;
    std::fs::write(&pending_path, &request_uuid)?;
    std::fs::write(identity_dir.join("pending_screenname"), &screenname)?;

    println!(
        "Check your email at {} and click the verification link.",
        email
    );
    println!("...then come back here...");

    let csr_code = poll_for_verification(server_url, &request_uuid).await?;

    println!("\nEmail verified! Generating keys...");

    complete_registration(
        server_url,
        identity_dir,
        &request_uuid,
        csr_code.as_deref(),
        &screenname,
        &pending_path,
    )
    .await
}

async fn resume_from_polling(
    server_url: &str,
    identity_dir: &Path,
    request_uuid: &str,
) -> Result<()> {
    let pending_path = identity_dir.join("pending_request");
    let screenname = std::fs::read_to_string(identity_dir.join("pending_screenname"))
        .unwrap_or_default()
        .trim()
        .to_string();

    let client = reqwest::Client::new();
    let url = format!("{}/api/verify/{}", server_url, request_uuid);
    let resp: serde_json::Value = client.get(&url).send().await?.json().await?;

    match resp["status"].as_str() {
        Some("verified") => {
            println!("Already verified! Completing registration...");
            let csr_code = resp["csr_code"].as_str().map(|s| s.to_string());
            complete_registration(
                server_url,
                identity_dir,
                request_uuid,
                csr_code.as_deref(),
                &screenname,
                &pending_path,
            )
            .await
        }
        Some("expired") => {
            std::fs::remove_file(&pending_path).ok();
            bail!("Previous verification expired. Please run again to start over.");
        }
        Some("pending") => {
            println!("Still waiting for email verification...");
            let csr_code = poll_for_verification(server_url, request_uuid).await?;
            complete_registration(
                server_url,
                identity_dir,
                request_uuid,
                csr_code.as_deref(),
                &screenname,
                &pending_path,
            )
            .await
        }
        _ => bail!("Unexpected status from server: {:?}", resp),
    }
}

async fn poll_for_verification(server_url: &str, request_uuid: &str) -> Result<Option<String>> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/verify/{}", server_url, request_uuid);

    for _ in 0..90 {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        let resp: serde_json::Value = client.get(&url).send().await?.json().await?;
        match resp["status"].as_str() {
            Some("verified") => {
                let csr_code = resp["csr_code"].as_str().map(|s| s.to_string());
                return Ok(csr_code);
            }
            Some("expired") => bail!("Verification expired. Please try again."),
            Some("pending") => {
                use std::io::Write;
                print!(".");
                std::io::stdout().flush().ok();
                continue;
            }
            _ => bail!("Unexpected status: {:?}", resp),
        }
    }
    bail!("Verification timed out after 15 minutes.")
}

async fn complete_registration(
    server_url: &str,
    identity_dir: &Path,
    request_uuid: &str,
    csr_code: Option<&str>,
    screenname: &str,
    pending_path: &Path,
) -> Result<()> {
    let (key_pem, csr_pem) = generate_keypair_and_csr()?;

    let mut csr_body = serde_json::json!({
        "request_uuid": request_uuid,
        "csr": csr_pem
    });
    if let Some(code) = csr_code {
        csr_body["csr_code"] = serde_json::Value::String(code.to_string());
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/csr", server_url))
        .json(&csr_body)
        .send()
        .await?;

    if !resp.status().is_success() {
        bail!("CSR submission failed: {}", resp.text().await?);
    }

    let body: serde_json::Value = resp.json().await?;
    let cert_pem = body["cert"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing cert in response"))?;
    let ca_pem = body["ca_cert"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing ca_cert in response"))?;
    let user_uuid = body["user_uuid"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing user_uuid in response"))?;

    crate::identity::credentials::save_credentials(
        identity_dir,
        &key_pem,
        cert_pem,
        ca_pem,
        user_uuid,
    )?;

    let e2e_public = crate::identity::e2e_keys::generate_and_store(identity_dir)?;

    let token = crate::auth::token::build_token(identity_dir, server_url)?;
    let upload_resp = client
        .post(format!("{}/api/e2e_key", server_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "public_key": base64::engine::general_purpose::STANDARD.encode(&e2e_public)
        }))
        .send()
        .await?;

    if !upload_resp.status().is_success() {
        eprintln!(
            "Warning: E2E key upload failed: {}",
            upload_resp.text().await?
        );
    }

    std::fs::write(identity_dir.join("screenname"), screenname)?;
    std::fs::remove_file(pending_path).ok();
    std::fs::remove_file(identity_dir.join("pending_screenname")).ok();

    println!("\nSetup complete! Identity saved to {:?}", identity_dir);
    println!("User UUID: {}", user_uuid);

    Ok(())
}

fn generate_keypair_and_csr() -> Result<(String, String)> {
    use rcgen::{CertificateParams, KeyPair, PKCS_RSA_SHA256};

    let key_pair = KeyPair::generate_for(&PKCS_RSA_SHA256)?;
    let key_pem = key_pair.serialize_pem();

    let params = CertificateParams::new(vec![])?;
    let csr = params.serialize_request(&key_pair)?;
    let csr_pem = csr.pem()?;

    Ok((key_pem, csr_pem))
}
