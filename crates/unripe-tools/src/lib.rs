pub mod bash;
pub mod read_file;
pub mod write_file;

pub use bash::BashTool;
pub use read_file::ReadFileTool;
pub use write_file::WriteFileTool;

use unripe_core::tool::Tool;

/// Create the default set of built-in tools
pub fn builtin_tools(bash_timeout_secs: u64) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFileTool),
        Box::new(WriteFileTool),
        Box::new(BashTool::new(bash_timeout_secs)),
    ]
}
