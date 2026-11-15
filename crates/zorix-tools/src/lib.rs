//! ZORIX tools: built-in demonstration tools.
//!
//! They exist to prove the tool architecture end to end, and are small on purpose:
//! `clock.now`, `math.evaluate`, `text.stats`, `notes.put`, `notes.get`, and the file tools `workspace.list`,
//! `workspace.read` and `workspace.write`, which are confined to one folder.

mod math;
mod notes;
mod simple;
pub mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

pub use math::{MathTool, evaluate};
pub use notes::{GetNoteTool, Notebook, PutNoteTool};
pub use simple::{ClockTool, TextStatsTool};
pub use workspace::{ListTool, ReadTool, RelativePath, WriteTool};
use zorix_core::{Result, Tool, ToolRegistry};

/// Where the built-in tools keep what they need.
#[derive(Clone, Debug)]
pub struct BuiltinConfig {
    /// The notes file.
    pub notes_file: PathBuf,
}

/// Every built-in tool.
pub fn builtin_tools(config: &BuiltinConfig) -> Vec<Arc<dyn Tool>> {
    let notebook = Notebook::new(config.notes_file.clone());
    vec![
        Arc::new(ClockTool::new()),
        Arc::new(MathTool::new()),
        Arc::new(TextStatsTool::new()),
        Arc::new(ListTool::new()),
        Arc::new(ReadTool::new()),
        Arc::new(WriteTool::new()),
        Arc::new(PutNoteTool::new(notebook.clone())),
        Arc::new(GetNoteTool::new(notebook)),
    ]
}

/// Register every built-in tool.
pub fn register_builtin(registry: &dyn ToolRegistry, config: &BuiltinConfig) -> Result<()> {
    for tool in builtin_tools(config) {
        registry.register(tool)?;
    }
    Ok(())
}
