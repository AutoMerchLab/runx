use std::fs;
use std::path::Path;

use super::error::ToolCatalogError;

pub(crate) struct ValidatedToolDocument {
    pub(crate) source: String,
    pub(crate) tool: runx_parser::ValidatedTool,
}

pub(crate) fn read(path: &Path) -> Result<runx_parser::ValidatedTool, ToolCatalogError> {
    read_document(path).map(|document| document.tool)
}

pub(crate) fn read_document(path: &Path) -> Result<ValidatedToolDocument, ToolCatalogError> {
    let source = fs::read_to_string(path)
        .map_err(|error| ToolCatalogError::io("reading tool manifest", path, error))?;
    let tool = parse(path, &source)?;
    Ok(ValidatedToolDocument { source, tool })
}

pub(crate) fn parse(
    path: &Path,
    source: &str,
) -> Result<runx_parser::ValidatedTool, ToolCatalogError> {
    let raw = runx_parser::parse_tool_manifest_json(source).map_err(|error| {
        ToolCatalogError::InvalidManifest {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    runx_parser::validate_tool_manifest(raw).map_err(|error| ToolCatalogError::InvalidManifest {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}
