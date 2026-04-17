use anyhow::Result;
use base64::Engine;
use chrono::Utc;
use std::path::Path;
use zeroize::Zeroizing;

pub fn build_token(identity_dir: &Path, server_host: &str) -> Result<String> {
    let timestamp = Utc::now().to_rfc3339();
    let message = format!("{}|{}", timestamp, server_host);

    let key_pem = Zeroizing::new(std::fs::read_to_string(
        identity_dir.join("private_key.pem"),
    )?);
    let key_pem_parsed = pem::parse(key_pem.as_bytes())?;

    let key_pair = ring::signature::RsaKeyPair::from_pkcs8(key_pem_parsed.contents())
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// Generate a temporary identity directory with a real RSA keypair and dummy cert.
    fn make_identity_dir() -> tempfile::TempDir {
        use rcgen::{KeyPair, PKCS_RSA_SHA256};
        let dir = tempfile::tempdir().unwrap();
        let key_pair = KeyPair::generate_for(&PKCS_RSA_SHA256).unwrap();
        std::fs::write(dir.path().join("private_key.pem"), key_pair.serialize_pem()).unwrap();
        std::fs::write(
            dir.path().join("cert.pem"),
            "-----BEGIN CERTIFICATE-----\nZmFrZQ==\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn test_token_has_three_parts() {
        let dir = make_identity_dir();
        let token = build_token(dir.path(), "https://example.com").unwrap();
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_token_message_contains_server_host() {
        let dir = make_identity_dir();
        let server = "https://example.com";
        let token = build_token(dir.path(), server).unwrap();

        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let message_part = token.split('.').next().unwrap();
        let message = String::from_utf8(b64.decode(message_part).unwrap()).unwrap();
        assert!(
            message.contains(server),
            "message should contain server host"
        );
        assert!(message.contains('|'), "message should be timestamp|host");
    }

    #[test]
    fn test_token_cert_matches_file() {
        let dir = make_identity_dir();
        let token = build_token(dir.path(), "https://example.com").unwrap();

        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let cert_part = token.split('.').nth(2).unwrap();
        let decoded = String::from_utf8(b64.decode(cert_part).unwrap()).unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join("cert.pem")).unwrap();
        assert_eq!(decoded, on_disk);
    }
}
