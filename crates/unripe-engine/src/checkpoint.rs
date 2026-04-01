use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Stores file backups before edits so they can be undone.
/// Each checkpoint maps a file path to its content before modification.
#[derive(Debug, Default)]
pub struct CheckpointStore {
    /// Stack of checkpoints. Each checkpoint is a set of file backups.
    checkpoints: Vec<Checkpoint>,
}

#[derive(Debug)]
struct Checkpoint {
    label: String,
    files: HashMap<PathBuf, Option<Vec<u8>>>, // None = file didn't exist
}

impl CheckpointStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Save a checkpoint before a tool modifies files.
    /// Call this with the file paths that are about to be modified.
    pub fn save(&mut self, label: &str, paths: &[PathBuf]) {
        let mut files = HashMap::new();
        for path in paths {
            let content = std::fs::read(path).ok();
            files.insert(path.clone(), content);
        }
        self.checkpoints.push(Checkpoint {
            label: label.to_string(),
            files,
        });
    }

    /// Undo the most recent checkpoint. Restores all files to their pre-edit state.
    /// Returns the label of the undone checkpoint, or None if no checkpoints exist.
    pub fn undo(&mut self) -> Option<String> {
        let checkpoint = self.checkpoints.pop()?;
        for (path, content) in &checkpoint.files {
            match content {
                Some(bytes) => {
                    // Restore original content
                    let _ = std::fs::write(path, bytes);
                }
                None => {
                    // File didn't exist before, remove it
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        Some(checkpoint.label)
    }

    /// Number of available checkpoints
    pub fn len(&self) -> usize {
        self.checkpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty()
    }

    /// List checkpoint labels (most recent first)
    pub fn labels(&self) -> Vec<&str> {
        self.checkpoints
            .iter()
            .rev()
            .map(|c| c.label.as_str())
            .collect()
    }
}

/// Infer which file paths a tool call will modify, for checkpointing.
pub fn tool_modified_paths(
    tool_name: &str,
    input: &serde_json::Value,
    project_root: &Path,
) -> Vec<PathBuf> {
    match tool_name {
        "write_file" | "edit_file" => {
            if let Some(path_str) = input.get("path").and_then(|v| v.as_str()) {
                let path = if PathBuf::from(path_str).is_absolute() {
                    PathBuf::from(path_str)
                } else {
                    project_root.join(path_str)
                };
                vec![path]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_save_and_undo() {
        let dir = std::env::temp_dir().join("unripe-test-checkpoint");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file = dir.join("test.txt");
        std::fs::write(&file, "original").unwrap();

        let mut store = CheckpointStore::new();
        store.save("edit test.txt", &[file.clone()]);

        // Simulate edit
        std::fs::write(&file, "modified").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "modified");

        // Undo
        let label = store.undo().unwrap();
        assert_eq!(label, "edit test.txt");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_undo_new_file() {
        let dir = std::env::temp_dir().join("unripe-test-checkpoint-new");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file = dir.join("new.txt");

        let mut store = CheckpointStore::new();
        store.save("write new.txt", &[file.clone()]);

        // Simulate write
        std::fs::write(&file, "new content").unwrap();
        assert!(file.exists());

        // Undo removes the file
        store.undo();
        assert!(!file.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_multiple_undos() {
        let dir = std::env::temp_dir().join("unripe-test-checkpoint-multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let file = dir.join("test.txt");
        std::fs::write(&file, "v1").unwrap();

        let mut store = CheckpointStore::new();

        // First edit
        store.save("edit 1", &[file.clone()]);
        std::fs::write(&file, "v2").unwrap();

        // Second edit
        store.save("edit 2", &[file.clone()]);
        std::fs::write(&file, "v3").unwrap();

        assert_eq!(store.len(), 2);

        // Undo second
        store.undo();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2");

        // Undo first
        store.undo();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v1");

        assert!(store.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_checkpoint_empty_undo() {
        let mut store = CheckpointStore::new();
        assert!(store.undo().is_none());
    }

    #[test]
    fn test_tool_modified_paths() {
        let root = PathBuf::from("/project");

        let paths = tool_modified_paths(
            "write_file",
            &serde_json::json!({"path": "src/main.rs", "content": "x"}),
            &root,
        );
        assert_eq!(paths, vec![PathBuf::from("/project/src/main.rs")]);

        let paths = tool_modified_paths(
            "edit_file",
            &serde_json::json!({"path": "/abs/file.rs", "old_string": "a", "new_string": "b"}),
            &root,
        );
        assert_eq!(paths, vec![PathBuf::from("/abs/file.rs")]);

        let paths = tool_modified_paths("bash", &serde_json::json!({"command": "ls"}), &root);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_checkpoint_labels() {
        let mut store = CheckpointStore::new();
        store.save("first", &[]);
        store.save("second", &[]);
        let labels = store.labels();
        assert_eq!(labels, vec!["second", "first"]);
    }
}
