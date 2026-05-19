/// The `fmt` subcommand — parse, validate, and normalize .facts files.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::lint;
use crate::parser;
use crate::project;
use crate::writer;

/// Options for the fmt command.
pub struct FmtOptions {
    /// Format a specific file instead of all *.facts files.
    pub file: Option<String>,
    /// When true, files are not modified; exit non-zero if any would change.
    pub check: bool,
}

/// Resolve which files to format.
fn resolve_files(root: &Path, opts: &FmtOptions) -> Result<Vec<PathBuf>> {
    if let Some(ref file) = opts.file {
        let (_, path) = project::resolve_file_arg(root, file);
        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }
        Ok(vec![path])
    } else {
        project::discover_fact_files(root)
    }
}

/// Run the fmt subcommand.
///
/// When `opts.check` is true, files are not modified. Returns `true` if all
/// files are already formatted (or would be in check mode), `false` if any
/// file would be changed.
pub fn run(opts: &FmtOptions) -> Result<bool> {
    let root = project::find_project_root()?;
    let files = resolve_files(&root, opts)?;

    if files.is_empty() {
        anyhow::bail!("no .facts files found in {}", root.display());
    }

    // Validate all files first — abort on any lint errors.
    let mut has_errors = false;
    for path in &files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(".facts");
        let diags = lint::lint_content(&content, filename);
        for diag in &diags {
            if diag.severity == lint::Severity::Error {
                let location = if let Some(line) = diag.line {
                    format!("{}:{}", diag.file, line)
                } else {
                    diag.file.clone()
                };
                eprintln!("{location}: error: {}", diag.message);
                has_errors = true;
            }
        }
    }
    if has_errors {
        anyhow::bail!("fmt aborted — fix lint errors first");
    }

    // Parse and write back each file.
    let mut would_change = false;
    for path in &files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(".facts");
        let sheet = parser::parse(&content, filename)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let output = writer::write(&sheet);
        if output != content {
            would_change = true;
            if opts.check {
                eprintln!("would reformat {}", path.display());
            } else {
                std::fs::write(path, &output)
                    .with_context(|| format!("failed to write {}", path.display()))?;
            }
        }
    }

    Ok(!would_change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_files_all() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".facts"), "- a\n").unwrap();
        fs::write(dir.path().join("api.facts"), "- b\n").unwrap();

        let opts = FmtOptions {
            file: None,
            check: false,
        };
        let files = resolve_files(dir.path(), &opts).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_resolve_files_explicit() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".facts"), "- a\n").unwrap();
        fs::write(dir.path().join("api.facts"), "- b\n").unwrap();

        let opts = FmtOptions {
            file: Some("api".to_string()),
            check: false,
        };
        let files = resolve_files(dir.path(), &opts).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().unwrap().to_str().unwrap(),
            "api.facts"
        );
    }

    #[test]
    fn test_resolve_files_explicit_with_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("api.facts"), "- b\n").unwrap();

        let opts = FmtOptions {
            file: Some("api.facts".to_string()),
            check: false,
        };
        let files = resolve_files(dir.path(), &opts).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().unwrap().to_str().unwrap(),
            "api.facts"
        );
    }

    #[test]
    fn test_resolve_files_missing_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".facts"), "- a\n").unwrap();

        let opts = FmtOptions {
            file: Some("nonexistent".to_string()),
            check: false,
        };
        let result = resolve_files(dir.path(), &opts);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("file not found"));
        assert!(err.contains("nonexistent.facts"));
    }
}
