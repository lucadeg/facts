/// The `config` subcommand — manage the .config fact in .facts.
use std::path::Path;

use anyhow::{Context, Result};

use crate::id;
use crate::locate::{self, FactLocation};
use crate::lock::FileLock;
use crate::model::{Fact, FactSheet};
use crate::parser;
use crate::project;
use crate::writer;

const CONFIG_ID: &str = ".config";
const CONFIG_LABEL: &str = "facts configuration";

pub enum ConfigAction {
    Get { key: String },
    Set { key: String, value: String },
    Show,
    Rm { key: String },
}

pub fn run(action: &ConfigAction) -> Result<()> {
    let root = project::find_project_root()?;
    run_in(action, &root)
}

fn run_in(action: &ConfigAction, root: &Path) -> Result<()> {
    match action {
        ConfigAction::Get { key } => config_get(root, key),
        ConfigAction::Set { key, value } => config_set(root, key, value),
        ConfigAction::Show => config_show(root),
        ConfigAction::Rm { key } => config_rm(root, key),
    }
}

fn config_get(root: &Path, key: &str) -> Result<()> {
    let (_, fact) = find_config_fact(root)?;
    for (k, v) in &fact.extra {
        if k == key {
            println!("{v}");
            return Ok(());
        }
    }
    anyhow::bail!("config key '{key}' not found");
}

fn config_set(root: &Path, key: &str, value: &str) -> Result<()> {
    let _lock = FileLock::acquire(root)?;
    let facts_path = root.join(".facts");

    let mut sheet = if facts_path.exists() {
        let content = std::fs::read_to_string(&facts_path)
            .with_context(|| format!("failed to read {}", facts_path.display()))?;
        parser::parse(&content, ".facts")?
    } else {
        FactSheet {
            filename: ".facts".to_string(),
            preamble: Vec::new(),
            sections: Vec::new(),
        }
    };

    match find_config_location(&sheet) {
        Some(location) => {
            let fact = locate::get_fact_mut(&mut sheet, &location);
            let mut found = false;
            for (k, v) in &mut fact.extra {
                if k == key {
                    *v = value.to_string();
                    found = true;
                    break;
                }
            }
            if !found {
                fact.extra.push((key.to_string(), value.to_string()));
            }
            fact.raw = writer::fact_to_raw(fact);
        }
        None => {
            let mut fact = Fact {
                explicit_id: Some(CONFIG_ID.to_string()),
                label: CONFIG_LABEL.to_string(),
                command: None,
                tags: Vec::new(),
                is_plain: false,
                raw: String::new(),
                blank_lines_before: 0,
                extra: vec![(key.to_string(), value.to_string())],
            };
            fact.raw = writer::fact_to_raw(&fact);
            sheet.preamble.push(fact);
        }
    }

    let output = writer::write(&sheet);
    std::fs::write(&facts_path, &output)
        .with_context(|| format!("failed to write {}", facts_path.display()))?;
    Ok(())
}

fn config_show(root: &Path) -> Result<()> {
    let (_, fact) = find_config_fact(root)?;
    for (k, v) in &fact.extra {
        println!("{k}: {v}");
    }
    Ok(())
}

fn config_rm(root: &Path, key: &str) -> Result<()> {
    let _lock = FileLock::acquire(root)?;
    let facts_path = root.join(".facts");

    let content = std::fs::read_to_string(&facts_path)
        .with_context(|| format!("failed to read {}", facts_path.display()))?;
    let mut sheet = parser::parse(&content, ".facts")?;

    let location = find_config_location(&sheet)
        .ok_or_else(|| anyhow::anyhow!("no .config fact found"))?;

    let fact = locate::get_fact_mut(&mut sheet, &location);
    let before = fact.extra.len();
    fact.extra.retain(|(k, _)| k != key);
    if fact.extra.len() == before {
        anyhow::bail!("config key '{key}' not found");
    }
    fact.raw = writer::fact_to_raw(fact);

    let output = writer::write(&sheet);
    std::fs::write(&facts_path, &output)
        .with_context(|| format!("failed to write {}", facts_path.display()))?;
    Ok(())
}

/// Read the .config fact value for a given key (used by other modules).
pub fn read_config_value(root: &Path, key: &str) -> Option<String> {
    let (_, fact) = find_config_fact(root).ok()?;
    fact.extra
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn find_config_fact(root: &Path) -> Result<(FactSheet, Fact)> {
    let facts_path = root.join(".facts");
    if !facts_path.exists() {
        anyhow::bail!("no .config fact found");
    }
    let content = std::fs::read_to_string(&facts_path)
        .with_context(|| format!("failed to read {}", facts_path.display()))?;
    let sheet = parser::parse(&content, ".facts")?;

    for (_, fact) in sheet.all_facts() {
        if fact.explicit_id.as_deref() == Some(CONFIG_ID) {
            return Ok((sheet.clone(), fact.clone()));
        }
    }
    anyhow::bail!("no .config fact found");
}

fn find_config_location(sheet: &FactSheet) -> Option<FactLocation> {
    let mut all_labels: Vec<(String, Option<String>)> = Vec::new();
    let mut locations: Vec<FactLocation> = Vec::new();

    for (i, fact) in sheet.preamble.iter().enumerate() {
        all_labels.push((fact.label.clone(), fact.explicit_id.clone()));
        locations.push(FactLocation::Preamble(i));
    }
    collect_config_locations(&sheet.sections, &[], &mut all_labels, &mut locations);

    let _assigned = id::assign_ids(&all_labels);

    for (i, (_, eid)) in all_labels.iter().enumerate() {
        if eid.as_deref() == Some(CONFIG_ID) {
            return Some(locations[i].clone());
        }
    }
    None
}

fn collect_config_locations(
    sections: &[crate::model::Section],
    parent: &[usize],
    labels: &mut Vec<(String, Option<String>)>,
    locations: &mut Vec<FactLocation>,
) {
    for (i, section) in sections.iter().enumerate() {
        let mut path = parent.to_vec();
        path.push(i);
        for (j, fact) in section.facts.iter().enumerate() {
            labels.push((fact.label.clone(), fact.explicit_id.clone()));
            locations.push(FactLocation::Section(path.clone(), j));
        }
        collect_config_locations(&section.children, &path, labels, locations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir(content: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        let facts_path = dir.path().join(".facts");
        fs::write(&facts_path, content).unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    #[test]
    fn test_config_set_creates_config() {
        let dir = setup_test_dir("- some fact\n");
        run_in(
            &ConfigAction::Set {
                key: "agent".to_string(),
                value: "claude -p %p".to_string(),
            },
            dir.path(),
        )
        .unwrap();

        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("id: .config"));
        assert!(content.contains("label: facts configuration"));
        assert!(content.contains("agent: claude -p %p"));
    }

    #[test]
    fn test_config_get() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: claude -p %p\n",
        );
        run_in(
            &ConfigAction::Get {
                key: "agent".to_string(),
            },
            dir.path(),
        )
        .unwrap();
    }

    #[test]
    fn test_config_get_missing_key() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: claude -p %p\n",
        );
        let result = run_in(
            &ConfigAction::Get {
                key: "nonexistent".to_string(),
            },
            dir.path(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_config_show() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: claude -p %p\n  batch: 5\n",
        );
        run_in(&ConfigAction::Show, dir.path()).unwrap();
    }

    #[test]
    fn test_config_rm() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: claude -p %p\n  batch: 5\n",
        );
        run_in(
            &ConfigAction::Rm {
                key: "batch".to_string(),
            },
            dir.path(),
        )
        .unwrap();

        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("agent: claude -p %p"));
        assert!(!content.contains("batch"));
    }

    #[test]
    fn test_config_rm_missing_key() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: claude -p %p\n",
        );
        let result = run_in(
            &ConfigAction::Rm {
                key: "nonexistent".to_string(),
            },
            dir.path(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_config_set_updates_existing() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: claude -p %p\n",
        );
        run_in(
            &ConfigAction::Set {
                key: "agent".to_string(),
                value: "codex exec %p".to_string(),
            },
            dir.path(),
        )
        .unwrap();

        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("agent: codex exec %p"));
        assert!(!content.contains("claude -p %p"));
    }

    #[test]
    fn test_config_set_creates_file() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();

        run_in(
            &ConfigAction::Set {
                key: "agent".to_string(),
                value: "claude -p %p".to_string(),
            },
            dir.path(),
        )
        .unwrap();

        let content = fs::read_to_string(dir.path().join(".facts")).unwrap();
        assert!(content.contains("id: .config"));
        assert!(content.contains("agent: claude -p %p"));
    }

    #[test]
    fn test_config_label_must_be_facts_configuration() {
        let dir = setup_test_dir(
            "- id: .config\n  label: facts configuration\n  agent: test\n",
        );
        let (_, fact) = find_config_fact(dir.path()).unwrap();
        assert_eq!(fact.label, CONFIG_LABEL);
    }
}
