use anyhow::Result;
use std::path::PathBuf;

pub fn identity_dir() -> PathBuf {
    let new_path = home_dir()
        .map(|h| h.join(".sqwok").join("identity"))
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".sqwok")
                .join("identity")
        });

    // Migrate from old platform-specific location (e.g. ~/Library/Application Support/chat.sqwok.sqwok/identity on Mac).
    if !new_path.exists() {
        if let Some(old_path) = directories::ProjectDirs::from("chat", "sqwok", "sqwok")
            .map(|p| p.data_local_dir().join("identity"))
        {
            if old_path.exists() {
                if let Some(parent) = new_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&old_path, &new_path);
            }
        }
    }

    new_path
}

pub fn server_url() -> String {
    std::env::var("SQWOK_SERVER").unwrap_or_else(|_| "https://sqwok.fixbase.io".to_string())
}

/// Returns the home directory, or an error if it cannot be determined.
pub fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))
}

/// Returns the sqwok chats directory (~/.sqwok/chats/).
pub fn chats_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".sqwok").join("chats"))
}

/// Returns the sqwok chat directory for a specific chat (~/.sqwok/chats/{uuid}/).
pub fn chat_dir(chat_uuid: &str) -> Result<PathBuf> {
    Ok(chats_dir()?.join(chat_uuid))
}
