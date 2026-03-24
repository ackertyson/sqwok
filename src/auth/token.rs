use anyhow::Result;
use base64::Engine;
use chrono::Utc;
use std::path::Path;

pub fn build_token(identity_dir: &Path, server_host: &str) -> Result<String> {
    let timestamp = Utc::now().to_rfc3339();
    let message = format!("{}|{}", timestamp, server_host);

    let key_pem = std::fs::read_to_string(identity_dir.join("private_key.pem"))?;
    let key_pem_parsed = pem::parse(key_pem.as_bytes())?;
    let key_der = key_pem_parsed.contents().to_vec();

    let key_pair = ring::signature::RsaKeyPair::from_pkcs8(&key_der)
        .map_err(|e| anyhow::anyhow!("Failed to load RSA key: {:?}", e))?;

    let mut signature = vec![0u8; key_pair.public().modulus_len()];
    let rng = ring::rand::SystemRandom::new();
    key_pair
        .sign(
            &ring::signature::RSA_PKCS1_SHA256,
            &rng,
            message.as_bytes(),
            &mut signature,
        )
        .map_err(|e| anyhow::anyhow!("Signing failed: {:?}", e))?;

    let cert_pem = std::fs::read_to_string(identity_dir.join("cert.pem"))?;

    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    Ok(format!(
        "{}.{}.{}",
        b64.encode(message.as_bytes()),
        b64.encode(&signature),
        b64.encode(cert_pem.as_bytes())
    ))
}
