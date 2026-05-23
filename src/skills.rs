use anyhow::{Result, bail};

use crate::init::{self, SKILLS};
use crate::project;
use std::collections::HashSet;
use std::path::PathBuf;

pub enum SkillsAction {
    List,
    ListAll,
    Show { name: String },
    Update,
}

pub fn run(action: &SkillsAction) -> Result<()> {
    match action {
        SkillsAction::List => list(),
        SkillsAction::ListAll => list_all(),
        SkillsAction::Show { name } => show(name),
        SkillsAction::Update => update(),
    }
}

fn list() -> Result<()> {
    for (name, content) in SKILLS {
        let desc = extract_description(content);
        println!("{name:20} {desc}");
    }
    println!();
    println!("Use `facts skills show <name>` to read a skill.");
    println!("Use `facts skills update` to install or update skills in your project.");
    Ok(())
}

fn show(name: &str) -> Result<()> {
    for (skill_name, content) in SKILLS {
        if *skill_name == name {
            print!("{content}");
            return Ok(());
        }
    }
    if let Some(content) = load_local_skill_content(name) {
        print!("{content}");
        return Ok(());
    }
    bail!(
        "unknown skill '{name}'. Run `facts skills` to see available skills (use `facts skills list-all` to discover skills from ~/.agents/skills and project .agents/skills)."
    )
}

fn update() -> Result<()> {
    let root = project::find_project_root()?;

    for (name, content) in SKILLS {
        init::install_skill(&root, name, content)?;
    }

    if init::is_claude_available(&root) {
        for (name, _) in SKILLS {
            init::link_skill_for_claude(&root, name)?;
        }
    }

    Ok(())
}

fn extract_description(content: &str) -> String {
    let mut in_frontmatter = false;
    let mut in_description = false;
    let mut parts = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            continue;
        }
        if trimmed.starts_with("description:") {
            in_description = true;
            let rest = trimmed.strip_prefix("description:").unwrap().trim();
            if !rest.is_empty() && rest != ">" && rest != "|" {
                parts.push(rest.to_string());
            }
            continue;
        }
        if in_description {
            if !trimmed.is_empty() && !trimmed.contains(':')
                || trimmed.starts_with(' ')
                || line.starts_with("  ")
            {
                parts.push(trimmed.to_string());
            } else {
                break;
            }
        }
    }

    let full = parts.join(" ");
    if full.len() > 72 {
        let mut end = 72;
        while end > 0 && !full.as_bytes()[end].is_ascii_whitespace() {
            end -= 1;
        }
        if end == 0 {
            end = 72;
        }
        format!("{}…", &full[..end].trim_end())
    } else {
        full
    }
}

// --- unified skills list bridge helpers (for tracequest iter15) ---

fn discover_local_skills() -> Vec<(String, String, String)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut results = Vec::new();

    // bundled first (prefer on name collision)
    for (name, content) in SKILLS {
        let n = name.to_string();
        if seen.insert(n.clone()) {
            let desc = extract_description(content);
            results.push((n, desc, "bundled".to_string()));
        }
    }

    // scan locations: global then project/local
    let mut locations: Vec<(PathBuf, String)> = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        locations.push((
            PathBuf::from(home).join(".agents/skills"),
            "global".to_string(),
        ));
    }
    let proj_or_cwd = if let Ok(root) = project::find_project_root() {
        Some(root.join(".agents/skills"))
    } else if let Ok(cwd) = std::env::current_dir() {
        Some(cwd.join(".agents/skills"))
    } else {
        None
    };
    if let Some(p) = proj_or_cwd {
        locations.push((p, "project".to_string()));
    }

    for (dir, src) in locations {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for ent in rd.flatten() {
                let pth = ent.path();
                if pth.is_dir()
                    && let Some(n) = pth.file_name().and_then(|s| s.to_str())
                {
                    let name = n.to_string();
                    if seen.contains(&name) {
                        continue;
                    }
                    let md = pth.join("SKILL.md");
                    if md.is_file()
                        && let Ok(content) = std::fs::read_to_string(&md)
                    {
                        let desc = extract_description(&content);
                        if seen.insert(name.clone()) {
                            results.push((name, desc, src.clone()));
                        }
                    }
                }
            }
        }
    }
    results
}

fn load_local_skill_content(name: &str) -> Option<String> {
    let mut locations: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        locations.push(
            PathBuf::from(home)
                .join(".agents/skills")
                .join(name)
                .join("SKILL.md"),
        );
    }
    if let Ok(root) = project::find_project_root() {
        locations.push(root.join(".agents/skills").join(name).join("SKILL.md"));
    } else if let Ok(cwd) = std::env::current_dir() {
        locations.push(cwd.join(".agents/skills").join(name).join("SKILL.md"));
    }
    for md in locations {
        if md.is_file()
            && let Ok(c) = std::fs::read_to_string(md)
        {
            return Some(c);
        }
    }
    None
}

fn list_all() -> Result<()> {
    for (name, desc, source) in discover_local_skills() {
        println!("{name:20} [{source:7}] {desc}");
    }
    println!();
    println!(
        "Use `facts skills show <name>` to read a skill (now unifies bundled + .agents/skills)."
    );
    println!(
        "Use `facts skills update` to install or update the bundled facts-* skills in your project."
    );
    Ok(())
}
