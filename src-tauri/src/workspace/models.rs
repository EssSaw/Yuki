use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    pub data_root: Option<String>,
    pub project_roots: Vec<String>,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            data_root: None,
            project_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub name: String,
    pub relative_path: String,
    pub is_directory: bool,
}
