use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::{Builder, TempDir};

use crate::errors::{Error, Result};

pub struct OutputSchemaFile {
    _dir: TempDir,
    path: PathBuf,
}

impl OutputSchemaFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

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
