use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::{Builder, TempDir};

use crate::errors::{Error, Result};

/// Temporary on-disk output schema file passed to `codex --output-schema`.
///
/// The underlying temporary directory is kept alive by this struct and cleaned
/// up automatically when dropped.
pub struct OutputSchemaFile {
    _dir: TempDir,
    path: PathBuf,
}

impl OutputSchemaFile {
    /// Returns the filesystem path of the generated schema file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Creates a temporary JSON schema file for structured output turns.
///
/// Returns `Ok(None)` when no schema is provided.
pub fn create_output_schema_file(schema: Option<&Value>) -> Result<Option<OutputSchemaFile>> {
    let Some(schema) = schema else {
        return Ok(None);
    };

    if !schema.is_object() {
        return Err(Error::InvalidOutputSchema(
            "output_schema must be a plain JSON object".to_string(),
        ));
    }

    let dir = Builder::new().prefix("codex-output-schema-").tempdir()?;
    let schema_path = dir.path().join("schema.json");
    std::fs::write(&schema_path, serde_json::to_vec(schema)?)?;

    Ok(Some(OutputSchemaFile {
        _dir: dir,
        path: schema_path,
    }))
}
