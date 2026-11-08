//! Hearth tools: confined file and shell capabilities.

mod fetch;
mod file_tool;
mod file_write;
mod search;
mod search_providers;
mod shell_tool;
mod tool;
mod workspace;

pub use fetch::{
    DeepSeekFetch, FetchError, FetchProvider, FetchResult, FirecrawlFetch, HttpFetch,
    JinaFetch, TinyFishFetch, WebFetchTool,
};
pub use file_tool::FileTool;
pub use file_write::FileWrite;
pub use search::{SearchError, SearchProvider, SearchResult, WebSearchTool};
pub use search_providers::{
    BraveSearch, DeepSeekSearch, ExaSearch, FirecrawlSearch, TinyFishSearch,
};
pub use shell_tool::ShellTool;
pub use tool::{Tool, ToolError, ToolOutput, ToolRegistry};
pub use workspace::{DirEntryInfo, Workspace};
