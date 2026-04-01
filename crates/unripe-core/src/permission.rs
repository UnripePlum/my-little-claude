use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the tool is about to do
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolAction {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    BashExec(String),
    NetworkRequest(String),
}

/// Permission decision from the gate
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    Allow,
    Deny(String),
    Ask(String),
}

/// Sync trait for permission decisions.
/// Returns Allow/Deny/Ask. The engine handles Ask asynchronously.
pub trait PermissionGate: Send + Sync {
    fn check(&self, tool_name: &str, action: &ToolAction) -> Permission;
}

/// Default permission policy implementing tiered security:
/// - read_file inside project: Allow
/// - read_file outside project: Ask
/// - write_file: always Ask, outside project root: Deny
/// - bash: always Ask
/// - network: Ask
pub struct DefaultPermissionGate {
    pub project_root: PathBuf,
}

impl DefaultPermissionGate {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    fn is_inside_project(&self, path: &Path) -> bool {
        let canonical_root = self
            .project_root
            .canonicalize()
            .unwrap_or_else(|_| self.project_root.clone());
        // Try canonicalizing the path directly (works if file exists)
        if let Ok(p) = path.canonicalize() {
            return p.starts_with(&canonical_root);
        }
        // File doesn't exist yet: canonicalize parent dir, then append filename
        if let Some(parent) = path.parent() {
            let canonical_parent = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            if let Some(file_name) = path.file_name() {
                return canonical_parent
                    .join(file_name)
                    .starts_with(&canonical_root);
            }
        }
        false
    }
}

impl PermissionGate for DefaultPermissionGate {
    fn check(&self, _tool_name: &str, action: &ToolAction) -> Permission {
        match action {
            ToolAction::FileRead(path) => {
                if self.is_inside_project(path) {
                    Permission::Allow
                } else {
                    Permission::Ask(format!("Read file outside project: {}", path.display()))
                }
            }
            ToolAction::FileWrite(path) => {
                if !self.is_inside_project(path) {
                    Permission::Deny(format!(
                        "Cannot write outside project root: {}",
                        path.display()
                    ))
                } else {
                    Permission::Ask(format!("Write file: {}", path.display()))
                }
            }
            ToolAction::BashExec(cmd) => Permission::Ask(format!("Execute command: {cmd}")),
            ToolAction::NetworkRequest(url) => {
                Permission::Ask(format!("Network request to: {url}"))
            }
        }
    }
}

/// Auto-approve gate for testing and CI
pub struct AutoApproveGate;

impl PermissionGate for AutoApproveGate {
    fn check(&self, _tool_name: &str, _action: &ToolAction) -> Permission {
        Permission::Allow
    }
}

/// Auto-deny gate for testing
pub struct AutoDenyGate;

impl PermissionGate for AutoDenyGate {
    fn check(&self, _tool_name: &str, _action: &ToolAction) -> Permission {
        Permission::Deny("All actions denied".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_gate(root: &Path) -> DefaultPermissionGate {
        DefaultPermissionGate::new(root)
    }

    #[test]
    fn test_file_read_inside_project_is_allow() {
        let dir = std::env::temp_dir().join("unripe-test-perm-read");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        fs::write(&file, "fn main() {}").unwrap();

        let gate = make_gate(&dir);
        let result = gate.check("read_file", &ToolAction::FileRead(file.clone()));
        assert_eq!(result, Permission::Allow);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_file_read_outside_project_is_ask() {
        let dir = std::env::temp_dir().join("unripe-test-perm-read-outside");
        fs::create_dir_all(&dir).unwrap();

        let gate = make_gate(&dir);
        let outside_path = PathBuf::from("/etc/hosts");
        let result = gate.check("read_file", &ToolAction::FileRead(outside_path));
        assert!(matches!(result, Permission::Ask(_)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_file_write_inside_project_is_ask() {
        let dir = std::env::temp_dir().join("unripe-test-perm-write");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("output.txt");

        let gate = make_gate(&dir);
        let result = gate.check("write_file", &ToolAction::FileWrite(file));
        assert!(matches!(result, Permission::Ask(_)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_file_write_outside_project_is_deny() {
        let dir = std::env::temp_dir().join("unripe-test-perm-write-outside");
        fs::create_dir_all(&dir).unwrap();

        let gate = make_gate(&dir);
        let result = gate.check(
            "write_file",
            &ToolAction::FileWrite(PathBuf::from("/etc/passwd")),
        );
        assert!(matches!(result, Permission::Deny(_)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_bash_exec_is_always_ask() {
        let dir = std::env::temp_dir().join("unripe-test-perm-bash");
        fs::create_dir_all(&dir).unwrap();

        let gate = make_gate(&dir);
        let result = gate.check("bash", &ToolAction::BashExec("ls -la".into()));
        assert!(matches!(result, Permission::Ask(_)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_network_request_is_ask() {
        let dir = std::env::temp_dir().join("unripe-test-perm-net");
        fs::create_dir_all(&dir).unwrap();

        let gate = make_gate(&dir);
        let result = gate.check(
            "fetch",
            &ToolAction::NetworkRequest("https://api.example.com".into()),
        );
        assert!(matches!(result, Permission::Ask(_)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_auto_approve_gate() {
        let gate = AutoApproveGate;
        let result = gate.check("bash", &ToolAction::BashExec("rm -rf /".into()));
        assert_eq!(result, Permission::Allow);
    }

    #[test]
    fn test_auto_deny_gate() {
        let gate = AutoDenyGate;
        let result = gate.check("read_file", &ToolAction::FileRead(PathBuf::from("main.rs")));
        assert!(matches!(result, Permission::Deny(_)));
    }
}
