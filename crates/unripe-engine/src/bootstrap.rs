use std::path::Path;

/// Load project context for the system prompt.
/// Reads CLAUDE.md, AGENTS.md, and custom context files if they exist.
pub fn load_project_context(project_root: &Path, extra_files: &[String]) -> String {
    let mut parts = Vec::new();

    // CLAUDE.md 4-level hierarchy (managed → user → project → local)
    // Later levels can override earlier ones. All are loaded for context.
    let claude_md_paths: Vec<(String, std::path::PathBuf)> = vec![
        // 1. Managed: shipped with the tool
        (
            "CLAUDE.md (managed)".into(),
            project_root.join(".unripe/managed/CLAUDE.md"),
        ),
        // 2. User: global user preferences
        (
            "CLAUDE.md (user)".into(),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".unripe/CLAUDE.md"),
        ),
        // 3. Project: repository-level
        ("CLAUDE.md (project)".into(), project_root.join("CLAUDE.md")),
        // 4. Local: gitignored local overrides
        (
            "CLAUDE.md (local)".into(),
            project_root.join(".unripe/CLAUDE.md"),
        ),
    ];

    for (label, path) in &claude_md_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            if !content.trim().is_empty() {
                parts.push(format!("# {label}\n\n{content}"));
            }
        }
    }

    // Other standard context files
    let standard_files = ["AGENTS.md", ".unripe/context.md"];
    for filename in &standard_files {
        let path = project_root.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                parts.push(format!("# {filename}\n\n{content}"));
            }
        }
    }

    // Custom context files from config
    for filename in extra_files {
        let path = project_root.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                parts.push(format!("# {filename}\n\n{content}"));
            }
        }
    }

    // Git branch detection
    if let Some(branch) = detect_git_branch(project_root) {
        parts.push(format!("Current git branch: {branch}"));
    }

    parts.join("\n\n---\n\n")
}

fn detect_git_branch(project_root: &Path) -> Option<String> {
    let head_path = project_root.join(".git/HEAD");
    let content = std::fs::read_to_string(head_path).ok()?;
    let trimmed = content.trim();
    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
        Some(branch.to_string())
    } else {
        // Detached HEAD
        Some(trimmed[..8.min(trimmed.len())].to_string())
    }
}

/// Build the system prompt from project context
pub fn build_system_prompt(project_root: &Path, extra_files: &[String]) -> String {
    let context = load_project_context(project_root, extra_files);

    let mut prompt = String::from(
        "You are a coding assistant. You can read files, write files, and execute bash commands.\n\
         Work in the user's project directory. Be concise and direct.\n",
    );

    if !context.is_empty() {
        prompt.push_str("\n## Project Context\n\n");
        prompt.push_str(&context);
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_context_empty_dir() {
        let dir = std::env::temp_dir().join("unripe-test-bootstrap-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let context = load_project_context(&dir, &[]);
        assert!(context.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_context_with_claude_md() {
        let dir = std::env::temp_dir().join("unripe-test-bootstrap-claude");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "# Project Rules\nUse Rust.").unwrap();

        let context = load_project_context(&dir, &[]);
        assert!(context.contains("CLAUDE.md"));
        assert!(context.contains("Use Rust"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_context_with_custom_files() {
        let dir = std::env::temp_dir().join("unripe-test-bootstrap-custom");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("NOTES.md"), "Custom notes").unwrap();

        let context = load_project_context(&dir, &["NOTES.md".into()]);
        assert!(context.contains("Custom notes"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_context_with_git_branch() {
        let dir = std::env::temp_dir().join("unripe-test-bootstrap-git");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        let context = load_project_context(&dir, &[]);
        assert!(context.contains("main"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_build_system_prompt() {
        let dir = std::env::temp_dir().join("unripe-test-bootstrap-prompt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "Always use async.").unwrap();

        let prompt = build_system_prompt(&dir, &[]);
        assert!(prompt.contains("coding assistant"));
        assert!(prompt.contains("Always use async"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
