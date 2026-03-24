use anyhow::Result;
use directories::ProjectDirs;
use std::path::PathBuf;

pub fn identity_dir() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("chat", "sqwok", "sqwok") {
        proj_dirs.data_local_dir().join("identity")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".sqwok")
            .join("identity")
    }
}

pub fn server_url() -> String {
    std::env::var("SQWOK_SERVER").unwrap_or_else(|_| "http://localhost:4000".to_string())
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
