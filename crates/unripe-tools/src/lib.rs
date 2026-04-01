pub mod bash;
pub mod edit_file;
pub mod glob;
pub mod grep;
pub mod read_file;
pub mod web_fetch;
pub mod web_search;
pub mod write_file;

pub use bash::BashTool;
pub use edit_file::EditFileTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use read_file::ReadFileTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;
pub use write_file::WriteFileTool;

use unripe_core::tool::Tool;

/// Create the default set of built-in tools
pub fn builtin_tools(bash_timeout_secs: u64) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFileTool),
        Box::new(WriteFileTool),
        Box::new(EditFileTool),
        Box::new(BashTool::new(bash_timeout_secs)),
        Box::new(GlobTool),
        Box::new(GrepTool),
        Box::new(WebFetchTool::new()),
        Box::new(WebSearchTool::new()),
    ]
}
