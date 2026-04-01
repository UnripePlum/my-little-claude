use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::message::Message;

/// A conversation session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub turn_count: u32,
    #[serde(default)]
    pub token_estimate: u64,
}

impl Session {
    pub fn new(provider: &str, model: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
            provider: provider.to_string(),
            model: model.to_string(),
            turn_count: 0,
            token_estimate: 0,
        }
    }

    /// Add a message and update token estimate (rough: 4 chars per token)
    pub fn add_message(&mut self, message: Message) {
        let text = message.text_content();
        self.token_estimate += (text.len() as u64) / 4;
        self.messages.push(message);
    }

    /// Increment turn count
    pub fn increment_turn(&mut self) {
        self.turn_count += 1;
    }

    /// Truncate old messages to fit within limits.
    /// Keeps: system messages + last `keep_recent` messages.
    /// Old tool results are replaced with "[truncated]".
    pub fn truncate(&mut self, keep_recent: usize) {
        if self.messages.len() <= keep_recent {
            return;
        }

        let system_messages: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| m.role == crate::message::Role::System)
            .cloned()
            .collect();

        let non_system: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| m.role != crate::message::Role::System)
            .cloned()
            .collect();

        let recent_start = non_system.len().saturating_sub(keep_recent);
        let recent: Vec<Message> = non_system[recent_start..].to_vec();

        self.messages = system_messages;
        self.messages.extend(recent);

        // Recalculate token estimate
        self.token_estimate = self
            .messages
            .iter()
            .map(|m| (m.text_content().len() as u64) / 4)
            .sum();
    }
}

/// Handles saving and loading sessions from disk
pub struct SessionStore {
    pub base_dir: PathBuf,
}

impl SessionStore {
    pub fn new() -> anyhow::Result<Self> {
        let base_dir = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
            .join(".unripe")
            .join("sessions");
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    pub fn with_dir(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Save a session to disk
    pub fn save(&self, session: &Session) -> anyhow::Result<PathBuf> {
        let path = self.base_dir.join(format!("{}.json", session.id));
        let json = serde_json::to_string_pretty(session)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a session by ID
    pub fn load(&self, session_id: &str) -> anyhow::Result<Session> {
        let path = self.base_dir.join(format!("{session_id}.json"));
        self.load_from_path(&path)
    }

    /// Load the most recent session
    pub fn load_latest(&self) -> anyhow::Result<Session> {
        let mut entries: Vec<_> = std::fs::read_dir(&self.base_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .collect();

        entries.sort_by_key(|e| {
            std::cmp::Reverse(
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            )
        });

        let latest = entries
            .first()
            .ok_or_else(|| anyhow::anyhow!("No sessions found"))?;

        self.load_from_path(&latest.path())
    }

    fn load_from_path(&self, path: &Path) -> anyhow::Result<Session> {
        let json = std::fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&json).map_err(|e| {
            anyhow::anyhow!(
                "Session file corrupted ({}), start fresh: {e}",
                path.display()
            )
        })?;
        Ok(session)
    }

    /// List all session IDs
    pub fn list(&self) -> anyhow::Result<Vec<String>> {
        let entries: Vec<String> = std::fs::read_dir(&self.base_dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect();
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, Role};

    #[test]
    fn test_session_creation() {
        let session = Session::new("anthropic", "claude-sonnet-4-20250514");
        assert!(!session.id.is_empty());
        assert!(session.messages.is_empty());
        assert_eq!(session.provider, "anthropic");
        assert_eq!(session.model, "claude-sonnet-4-20250514");
        assert_eq!(session.turn_count, 0);
        assert_eq!(session.token_estimate, 0);
    }

    #[test]
    fn test_session_add_message() {
        let mut session = Session::new("ollama", "qwen2.5-coder:7b");
        session.add_message(Message::text(Role::User, "hello world"));
        assert_eq!(session.messages.len(), 1);
        assert!(session.token_estimate > 0);
    }

    #[test]
    fn test_session_increment_turn() {
        let mut session = Session::new("test", "test");
        assert_eq!(session.turn_count, 0);
        session.increment_turn();
        assert_eq!(session.turn_count, 1);
        session.increment_turn();
        assert_eq!(session.turn_count, 2);
    }

    #[test]
    fn test_session_truncation() {
        let mut session = Session::new("test", "test");
        session.add_message(Message::text(Role::System, "You are helpful."));
        for i in 0..20 {
            session.add_message(Message::text(Role::User, format!("Message {i}")));
            session.add_message(Message::text(Role::Assistant, format!("Reply {i}")));
        }
        assert_eq!(session.messages.len(), 41); // 1 system + 40 user/assistant

        session.truncate(10);
        // Should have: 1 system + 10 recent
        assert_eq!(session.messages.len(), 11);
        assert_eq!(session.messages[0].role, Role::System);
    }

    #[test]
    fn test_truncation_no_op_when_small() {
        let mut session = Session::new("test", "test");
        session.add_message(Message::text(Role::User, "hello"));
        session.add_message(Message::text(Role::Assistant, "hi"));
        session.truncate(10);
        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn test_session_serialization_roundtrip() {
        let mut session = Session::new("anthropic", "claude-sonnet-4-20250514");
        session.add_message(Message::text(Role::User, "test message"));
        session.increment_turn();

        let json = serde_json::to_string(&session).unwrap();
        let deserialized: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, session.id);
        assert_eq!(deserialized.messages.len(), 1);
        assert_eq!(deserialized.turn_count, 1);
        assert_eq!(deserialized.provider, "anthropic");
    }

    #[test]
    fn test_session_store_save_and_load() {
        let dir = std::env::temp_dir().join("unripe-test-session-store");
        std::fs::create_dir_all(&dir).unwrap();

        let store = SessionStore::with_dir(&dir);
        let mut session = Session::new("test", "model");
        session.add_message(Message::text(Role::User, "hello"));

        let path = store.save(&session).unwrap();
        assert!(path.exists());

        let loaded = store.load(&session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.messages.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_session_store_load_latest() {
        let dir = std::env::temp_dir().join("unripe-test-session-latest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let store = SessionStore::with_dir(&dir);

        let session1 = Session::new("test", "model1");
        store.save(&session1).unwrap();

        // Small delay to ensure different modification times
        std::thread::sleep(std::time::Duration::from_millis(50));

        let session2 = Session::new("test", "model2");
        store.save(&session2).unwrap();

        let latest = store.load_latest().unwrap();
        assert_eq!(latest.id, session2.id);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_session_store_corrupted_file() {
        let dir = std::env::temp_dir().join("unripe-test-session-corrupt");
        std::fs::create_dir_all(&dir).unwrap();

        let bad_path = dir.join("bad-session.json");
        std::fs::write(&bad_path, "not valid json {{{").unwrap();

        let store = SessionStore::with_dir(&dir);
        let result = store.load("bad-session");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("corrupted"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_session_store_list() {
        let dir = std::env::temp_dir().join("unripe-test-session-list");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let store = SessionStore::with_dir(&dir);
        let s1 = Session::new("test", "m");
        let s2 = Session::new("test", "m");
        store.save(&s1).unwrap();
        store.save(&s2).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
