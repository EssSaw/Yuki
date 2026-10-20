use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::workspace::explorer::{
    get_shareable_project_roots_impl, list_provider_project_directory_impl,
};
use crate::workspace::reader::{authorize_provider_file, read_local_file_chunk_impl};

pub const MAX_TOOL_ROUNDS: usize = 5;
pub const MAX_TOOLS_PER_ROUND: usize = 8;
pub const MAX_TOTAL_TOOL_CALLS: usize = 16;
pub const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;
pub const MAX_CUMULATIVE_TOOL_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 64 * 1024;

pub const TOOL_LIST_SHAREABLE_PROJECTS: &str = "list_shareable_projects";
pub const TOOL_LIST_PROJECT_DIRECTORY: &str = "list_project_directory";
pub const TOOL_READ_FILE_CHUNK: &str = "read_file_chunk";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub fn get_chat_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: TOOL_LIST_SHAREABLE_PROJECTS.to_string(),
                description: "Lists approved project root paths that currently have provider sharing enabled. Only projects in this list can be accessed with directory listing or file reading.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: TOOL_LIST_PROJECT_DIRECTORY.to_string(),
                description: "Lists immediate files and subdirectories in a granted project folder. Both the project root and relative path must be within an approved project with active provider sharing.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "project_root": {
                            "type": "string",
                            "description": "The approved project root path (from list_shareable_projects)."
                        },
                        "relative_path": {
                            "type": "string",
                            "description": "Optional subdirectory path relative to the project root. Defaults to empty for project root."
                        }
                    },
                    "required": ["project_root"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: TOOL_READ_FILE_CHUNK.to_string(),
                description: "Reads a bounded chunk of UTF-8 text from an authorized file inside a granted project folder. Returns the text chunk along with byte offsets and EOF status.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the file to read."
                        },
                        "offset": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Byte offset to begin reading from (default 0)."
                        },
                        "chunk_size": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 8192,
                            "description": "Maximum bytes to read in this chunk (clamped to at most 8KB, default 8KB)."
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
        },
    ]
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<RawToolCall>,
    total_arguments_bytes: usize,
}

impl ToolCallAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    ) -> Result<(), String> {
        if index >= MAX_TOOLS_PER_ROUND {
            return Err(format!(
                "Model exceeded maximum tool calls per round limit ({}).",
                MAX_TOOLS_PER_ROUND
            ));
        }

        while self.calls.len() <= index {
            self.calls.push(RawToolCall::default());
        }

        let call = &mut self.calls[index];
        if let Some(id_str) = id {
            call.id.push_str(&id_str);
        }
        if let Some(name_str) = name {
            call.name.push_str(&name_str);
        }
        if let Some(arg_chunk) = arguments_delta {
            self.total_arguments_bytes += arg_chunk.len();
            if self.total_arguments_bytes > MAX_TOOL_ARGUMENTS_BYTES {
                return Err("Tool arguments exceeded maximum allowed size (64KB).".to_string());
            }
            call.arguments.push_str(&arg_chunk);
        }

        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    pub fn into_calls(self) -> Vec<RawToolCall> {
        self.calls
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListProjectDirectoryArgs {
    project_root: String,
    #[serde(default)]
    relative_path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFileChunkArgs {
    path: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    chunk_size: Option<u32>,
}

/// Executes a requested tool against the authorized workspace.
/// All arguments are treated as untrusted and strictly validated.
/// Errors are serialized as structured JSON error responses for the model.
pub fn execute_tool(config_dir: &Path, name: &str, arguments_json: &str) -> String {
    match name {
        TOOL_LIST_SHAREABLE_PROJECTS => match get_shareable_project_roots_impl(config_dir) {
            Ok(projects) => serde_json::to_string(&serde_json::json!({
                "projects": projects
            }))
            .unwrap_or_else(|_| "{\"error\":\"Failed to serialize project list.\"}".to_string()),
            Err(e) => serde_json::to_string(&serde_json::json!({
                "error": format!("Failed to read shareable projects: {e}")
            }))
            .unwrap_or_else(|_| "{\"error\":\"Failed to read shareable projects.\"}".to_string()),
        },
        TOOL_LIST_PROJECT_DIRECTORY => {
            let args: ListProjectDirectoryArgs = match serde_json::from_str(arguments_json) {
                Ok(a) => a,
                Err(_) => {
                    return serde_json::to_string(&serde_json::json!({
                        "error": "Invalid arguments for list_project_directory: malformed JSON or missing 'project_root' string."
                    }))
                    .unwrap();
                }
            };

            let trimmed_root = args.project_root.trim();
            if trimmed_root.is_empty() {
                return serde_json::to_string(&serde_json::json!({
                    "error": "project_root cannot be empty."
                }))
                .unwrap();
            }

            let rel_path = args.relative_path.as_deref().unwrap_or("");
            match list_provider_project_directory_impl(config_dir, trimmed_root, rel_path) {
                Ok(entries) => serde_json::to_string(&serde_json::json!({
                    "entries": entries
                }))
                .unwrap_or_else(|_| {
                    "{\"error\":\"Failed to serialize directory entries.\"}".to_string()
                }),
                Err(e) => serde_json::to_string(&serde_json::json!({
                    "error": e
                }))
                .unwrap_or_else(|_| "{\"error\":\"Directory listing rejected.\"}".to_string()),
            }
        }
        TOOL_READ_FILE_CHUNK => {
            let args: ReadFileChunkArgs = match serde_json::from_str(arguments_json) {
                Ok(a) => a,
                Err(_) => {
                    return serde_json::to_string(&serde_json::json!({
                        "error": "Invalid arguments for read_file_chunk: malformed JSON or missing 'path' string."
                    }))
                    .unwrap();
                }
            };

            let trimmed_path = args.path.trim();
            if trimmed_path.is_empty() {
                return serde_json::to_string(&serde_json::json!({
                    "error": "File path cannot be empty."
                }))
                .unwrap();
            }

            // 1. Authorize file for provider sharing
            let file_path = Path::new(trimmed_path);
            if let Err(e) = authorize_provider_file(config_dir, file_path) {
                return serde_json::to_string(&serde_json::json!({
                    "error": format!("Access denied: {e}")
                }))
                .unwrap();
            }

            // 2. Read bounded chunk
            let offset = args.offset.unwrap_or(0);
            match read_local_file_chunk_impl(config_dir, trimmed_path, offset, Some(args.chunk_size.unwrap_or(8192).min(8192))) {
                Ok(chunk) => serde_json::to_string(&serde_json::json!({
                    "path": chunk.path,
                    "text": chunk.text,
                    "offset": chunk.offset,
                    "nextOffset": chunk.next_offset,
                    "totalBytes": chunk.total_bytes,
                    "eof": chunk.eof
                }))
                .unwrap_or_else(|_| "{\"error\":\"Failed to serialize file chunk.\"}".to_string()),
                Err(e) => serde_json::to_string(&serde_json::json!({
                    "error": format!("Read failed: {e}")
                }))
                .unwrap_or_else(|_| "{\"error\":\"File read failed.\"}".to_string()),
            }
        }
        other => serde_json::to_string(&serde_json::json!({
            "error": format!("Unknown tool '{other}'. Only read-only project inspection tools are permitted.")
        }))
        .unwrap(),
    }
}

/// Re-authorizes sharing before each tool result is sent across the network boundary to the provider.
/// If sharing was revoked, replaces result content with an authorization error so no file or directory
/// data crosses the send boundary.
pub fn reauthorize_tool_result(
    config_dir: &Path,
    name: &str,
    arguments_json: &str,
    result_content: &mut String,
) {
    match name {
        TOOL_READ_FILE_CHUNK => {
            if let Ok(args) = serde_json::from_str::<ReadFileChunkArgs>(arguments_json) {
                let file_path = Path::new(args.path.trim());
                if let Err(e) = authorize_provider_file(config_dir, file_path) {
                    *result_content = serde_json::to_string(&serde_json::json!({
                        "error": format!("Provider sharing authorization failed before sending to provider: {e}")
                    }))
                    .unwrap();
                }
            }
        }
        TOOL_LIST_PROJECT_DIRECTORY => {
            if let Ok(args) = serde_json::from_str::<ListProjectDirectoryArgs>(arguments_json) {
                let rel_path = args.relative_path.as_deref().unwrap_or("");
                if let Err(e) = list_provider_project_directory_impl(
                    config_dir,
                    args.project_root.trim(),
                    rel_path,
                ) {
                    *result_content = serde_json::to_string(&serde_json::json!({
                        "error": format!("Provider sharing authorization failed before sending to provider: {e}")
                    }))
                    .unwrap();
                }
            }
        }
        TOOL_LIST_SHAREABLE_PROJECTS => {
            if let Ok(projects) = get_shareable_project_roots_impl(config_dir) {
                *result_content = serde_json::to_string(&serde_json::json!({
                    "projects": projects
                }))
                .unwrap();
            } else {
                *result_content = serde_json::to_string(&serde_json::json!({
                    "projects": []
                }))
                .unwrap();
            }
        }
        _ => {}
    }
}
