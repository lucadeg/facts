mod add;
mod agent;
mod check;
mod color;
mod config;
mod edit;
mod fmt;
mod get;
mod id;
mod init;
mod lint;
mod list;
mod locate;
mod lock;
mod model;
mod move_fact;
mod parser;
mod project;
mod remove;
mod skills;
mod tags;
mod tui;
mod uninit;
mod update;
mod writer;

use clap::{Parser, Subcommand};

/// A CLI for fact-driven development with coding agents.
#[derive(Parser)]
#[command(
    name = "facts",
    version,
    about,
    before_help = "\
Start here (for AI agents):\n  \
  facts skills show facts\n\n  \
  Skills ship with the CLI and include the full workflow, format spec,\n  \
  and command reference. Read the skill before using the CLI.\n\n  \
  skills [list]               List available (bundled facts-*) skills\n  \
  skills list-all             List ALL discoverable (bundled + ~/.agents + project .agents)\n  \
  skills show <name>          Read a skill (facts-* or any from .agents/skills)\n  \
  skills update               Install/update skills in the project\n\n  \
Common short aliases (all args passed through):\n  \
  ll = list --light     ls = list\n  \
  rm = remove\n  \
  at <id> <tag> = edit <id> --add-tag <tag>     rt <id> <tag> = edit <id> --remove-tag <tag>\
"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show facts in file order (default when no subcommand given).
    List {
        /// Filter by file name.
        #[arg(long)]
        file: Option<String>,

        /// Filter by section path.
        #[arg(long)]
        section: Option<String>,

        /// Show only facts with a validation command.
        #[arg(long)]
        has_command: bool,

        /// Show only manual facts (no validation command).
        #[arg(long)]
        manual: bool,

        /// Boolean tag filter expression (e.g. "mvp and not blocked").
        #[arg(long)]
        tags: Option<String>,

        /// Boolean search expression matched against section, label, and tags (e.g. "update and cli").
        #[arg(long)]
        search: Option<String>,

        /// Limit section nesting depth (0 = top-level only).
        #[arg(long)]
        depth: Option<usize>,

        /// Show markdown-like output with headings, bullets, and dimmed IDs.
        #[arg(long)]
        light: bool,

        /// Include config facts (dot-prefixed IDs like .config) in output.
        #[arg(long)]
        all: bool,
    },

    /// Run all command-facts, report pass/fail/manual.
    Check {
        /// Check only the fact(s) with these IDs.
        #[arg(long)]
        id: Vec<String>,

        /// Boolean tag filter expression (e.g. "mvp and not blocked").
        #[arg(long)]
        tags: Option<String>,

        /// Boolean search expression matched against section, label, and tags (e.g. "update and cli").
        #[arg(long)]
        search: Option<String>,

        /// Limit section nesting depth (0 = top-level only).
        #[arg(long)]
        depth: Option<usize>,

        /// Per-command timeout in seconds.
        #[arg(long)]
        timeout: Option<u64>,

        /// Agent for manual facts: a preset name (claude, codex, pi, droid, opencode, mi) or
        /// a shell command template with %p as the prompt placeholder (e.g. "claude -p %p").
        /// Without this flag, auto-detects by checking .config agent, then scanning PATH for presets.
        #[arg(long, conflicts_with = "no_agent")]
        agent: Option<String>,

        /// Suppress agent verification of manual facts entirely (skip auto-detection and .config).
        #[arg(long, conflicts_with = "agent")]
        no_agent: bool,

        /// Number of manual facts per agent batch (default: 10, configurable via `facts config set batch-size N`).
        #[arg(long)]
        batch_size: Option<usize>,

        /// Suppress all output except the exit code.
        #[arg(long, short)]
        quiet: bool,

        /// Show extra detail: agent prompts and raw output.
        #[arg(long, short)]
        verbose: bool,
    },

    /// Append a fact to a file and section.
    Add {
        /// The fact label text.
        label: String,

        /// Target section path (e.g. "cli/add"). Created if needed.
        #[arg(long)]
        section: Option<String>,

        /// Target .facts file (default: ".facts"). Created if needed.
        #[arg(long)]
        file: Option<String>,

        /// Validation command.
        #[arg(long)]
        command: Option<String>,

        /// Explicit ID override.
        #[arg(long)]
        id: Option<String>,

        /// Comma-separated tags (e.g. "mvp,core").
        #[arg(long)]
        tags: Option<String>,
    },

    /// Remove a fact by ID, outputs what was removed.
    Remove {
        /// The ID of the fact to remove.
        id: String,
    },

    /// Move a fact by ID to a different section or file.
    Move {
        /// The ID of the fact to move.
        id: String,

        /// Target section path (e.g. "cli/check"). Created if needed.
        #[arg(long)]
        section: Option<String>,

        /// Target .facts file (e.g. "api.facts").
        #[arg(long)]
        file: Option<String>,
    },

    /// Look up a single fact by ID and display its details.
    Get {
        /// The ID of the fact to look up.
        id: String,
    },

    /// Modify one or more facts by ID.
    Edit {
        /// The ID(s) of the fact(s) to edit.
        #[arg(num_args = 1..)]
        ids: Vec<String>,

        /// New label text.
        #[arg(long)]
        label: Option<String>,

        /// New validation command.
        #[arg(long)]
        command: Option<String>,

        /// New explicit ID.
        #[arg(long, name = "new-id")]
        new_id: Option<String>,

        /// New tags (comma-separated, e.g. "mvp,core"). Replaces all existing tags.
        #[arg(long, conflicts_with_all = ["add-tag", "remove-tag"])]
        tags: Option<String>,

        /// Add tags without removing existing ones (comma-separated).
        #[arg(long, name = "add-tag")]
        add_tag: Option<String>,

        /// Remove specific tags (comma-separated).
        #[arg(long, name = "remove-tag")]
        remove_tag: Option<String>,
    },

    /// Validate that fact sheets are parseable.
    Lint {
        /// Lint a specific file instead of all *.facts files.
        #[arg(long)]
        file: Option<String>,
    },

    /// Parse, validate, and normalize all .facts files.
    Fmt {
        /// Format a specific file instead of all *.facts files.
        #[arg(long)]
        file: Option<String>,

        /// Check formatting without modifying files; exit non-zero if any file
        /// would be reformatted.
        #[arg(long)]
        check: bool,
    },

    /// Scaffold a .facts file and install agent skills.
    Init {
        /// Name for the fact sheet (e.g. "api" creates api.facts). Omit for .facts.
        name: Option<String>,
    },

    /// Remove .facts file and agent skills installed by init.
    Uninit {
        /// Delete .facts even when it has content.
        #[arg(long)]
        force: bool,
    },

    /// Update facts to the latest version.
    Update,

    /// Manage agent skills. Use `show` to read a skill without installing.
    Skills {
        #[command(subcommand)]
        action: Option<SkillsCommand>,
    },

    /// Manage tool configuration (.config fact). Known keys: agent (preset name or
    /// command template), batch-size (facts per agent batch).
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },

    /// Browse and manage facts interactively.
    Tui,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Get a config value by key.
    Get {
        /// The config key to look up.
        key: String,
    },
    /// Set a config value (creates .config if needed). Known keys: agent (preset name like
    /// "claude" or command template like "claude -p %p"), batch-size (facts per agent batch).
    Set {
        /// The config key (e.g. "agent", "batch-size").
        key: String,
        /// The config value.
        value: String,
    },
    /// Show all config key-value pairs.
    Show,
    /// Show all config key-value pairs (alias for show).
    Ls,
    /// Remove a config key.
    Rm {
        /// The config key to remove.
        key: String,
    },
}

#[derive(Subcommand)]
enum SkillsCommand {
    /// Show available skills with descriptions.
    List,
    /// List *all* discoverable skills: the 4 bundled facts-* plus any from
    /// $HOME/.agents/skills/*/SKILL.md and <project>/.agents/skills/*/SKILL.md
    /// (with [source] tag and short description from frontmatter). This is the
    /// unified skills list bridge.
    ListAll,
    /// Print the full content of a skill.
    Show {
        /// Skill name (e.g. "facts", "facts-discover", or any .agents skill like "bugbash").
        name: String,
    },
    /// Install or update skills in the current project.
    Update,
}

/// Expand short aliases to their underlying commands, preserving and proxying
/// all additional arguments. This runs before clap parsing.
fn expand_aliases(mut args: Vec<String>) -> Vec<String> {
    if args.len() < 2 {
        return args;
    }
    let alias = args[1].as_str();
    match alias {
        "ll" => {
            args[1] = "list".to_string();
            args.insert(2, "--light".to_string());
        }
        "ls" => {
            args[1] = "list".to_string();
        }
        "rm" => {
            args[1] = "remove".to_string();
        }
        "at" | "rt" => {
            // Support `facts at --help` and `facts rt --help` by delegating to edit
            let has_help = args.iter().any(|a| a == "--help" || a == "-h");
            if has_help && args.len() <= 3 {
                // at --help or at <something> --help (simple cases) -> edit --help
                args[1] = "edit".to_string();
            } else if args.len() >= 4 {
                // Rewrite: at/rt <ID>... <TAG> [extra...]  ->  edit <ID>... --add-tag/--remove-tag <TAG> [extra...]
                // The tag is the last *bare* (non-flag) token among the leading positionals (before any --flag).
                let alias_cmd = args.remove(1); // remove "at" or "rt"
                let tail: Vec<String> = args.drain(1..).collect(); // now args has only [bin]
                // Find the first flag (starts with '-') — everything before it is the ID+tag positionals.
                let first_flag = tail.iter().position(|a| a.starts_with('-'));
                let (prefix, after) = match first_flag {
                    Some(i) => (&tail[..i], tail[i..].to_vec()),
                    None => (tail.as_slice(), vec![]),
                };
                if prefix.len() >= 2 {
                    // last bare in prefix is the tag, earlier ones are IDs
                    let tag = prefix.last().unwrap().clone();
                    let ids: Vec<String> = prefix[..prefix.len() - 1].to_vec();
                    let flag = if alias_cmd == "at" {
                        "--add-tag"
                    } else {
                        "--remove-tag"
                    };
                    args.push("edit".to_string());
                    args.extend(ids);
                    args.push(flag.to_string());
                    args.push(tag);
                    args.extend(after);
                } else {
                    // not enough leading bare words for "ID(s) TAG", restore (unknown subcommand)
                    args.extend(tail);
                }
            }
            // else: not enough args and no --help, leave "at"/"rt" in place -> clap unknown subcommand
        }
        _ => {}
    }
    args
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let expanded = expand_aliases(args);
    let cli = Cli::parse_from(expanded);

    match cli.command {
        Some(Command::List {
            file,
            section,
            has_command,
            manual,
            tags,
            search,
            depth,
            light,
            all,
        }) => {
            let opts = list::ListOptions {
                file_filter: file,
                section_filter: section,
                has_command,
                manual,
                tags_expr: tags,
                search_expr: search,
                depth,
                light,
                all,
            };
            let had_output = list::run(&opts)?;
            if !had_output {
                std::process::exit(1);
            }
        }
        Some(Command::Check {
            id,
            tags,
            search,
            depth,
            timeout,
            agent,
            no_agent,
            batch_size,
            quiet,
            verbose,
        }) => {
            let opts = check::CheckOptions {
                id_filter: if id.is_empty() { None } else { Some(id) },
                tags_expr: tags,
                search_expr: search,
                depth,
                timeout,
                agent,
                no_agent,
                batch_size,
                quiet,
                verbose,
            };
            let all_passed = check::run(&opts)?;
            if !all_passed {
                std::process::exit(1);
            }
        }
        Some(Command::Add {
            label,
            section,
            file,
            command,
            id,
            tags,
        }) => {
            let tags = match tags {
                Some(t) => add::parse_tags(&t)?,
                None => Vec::new(),
            };
            let opts = add::AddOptions {
                label,
                file,
                section,
                command,
                id,
                tags,
            };
            let id = add::run(&opts)?;
            println!("{id}");
        }
        Some(Command::Remove { id }) => {
            remove::run(&id)?;
        }
        Some(Command::Move { id, section, file }) => {
            let opts = move_fact::MoveOptions {
                target_id: id,
                target_section: section,
                target_file: file,
            };
            move_fact::run(&opts)?;
        }
        Some(Command::Get { id }) => {
            get::run(&id)?;
        }
        Some(Command::Edit {
            ids,
            label,
            command,
            new_id,
            tags,
            add_tag,
            remove_tag,
        }) => {
            let tags = match tags {
                Some(t) => Some(add::parse_tags(&t)?),
                None => None,
            };
            let add_tags = match add_tag {
                Some(t) => Some(add::parse_tags(&t)?),
                None => None,
            };
            let remove_tags = match remove_tag {
                Some(t) => Some(add::parse_tags(&t)?),
                None => None,
            };
            let opts = edit::EditOptions {
                target_ids: ids,
                label,
                command,
                new_id,
                tags,
                add_tags,
                remove_tags,
            };
            edit::run(&opts)?;
        }
        Some(Command::Lint { file }) => {
            let opts = lint::LintOptions { file };
            let all_passed = lint::run(&opts)?;
            if !all_passed {
                std::process::exit(1);
            }
        }
        Some(Command::Fmt { file, check }) => {
            let opts = fmt::FmtOptions { file, check };
            let all_formatted = fmt::run(&opts)?;
            if check && !all_formatted {
                std::process::exit(1);
            }
        }
        Some(Command::Init { name }) => {
            init::run(name.as_deref())?;
        }
        Some(Command::Uninit { force }) => {
            uninit::run(force)?;
        }
        Some(Command::Update) => {
            update::run()?;
        }
        Some(Command::Skills { action }) => {
            let action = match action {
                Some(SkillsCommand::Show { name }) => skills::SkillsAction::Show { name },
                Some(SkillsCommand::Update) => skills::SkillsAction::Update,
                Some(SkillsCommand::ListAll) => skills::SkillsAction::ListAll,
                Some(SkillsCommand::List) | None => skills::SkillsAction::List,
            };
            skills::run(&action)?;
        }
        Some(Command::Config { action }) => {
            let action = match action {
                ConfigCommand::Get { key } => config::ConfigAction::Get { key },
                ConfigCommand::Set { key, value } => config::ConfigAction::Set { key, value },
                ConfigCommand::Show | ConfigCommand::Ls => config::ConfigAction::Show,
                ConfigCommand::Rm { key } => config::ConfigAction::Rm { key },
            };
            config::run(&action)?;
        }
        Some(Command::Tui) => {
            tui::run()?;
        }
        None => {
            let opts = list::ListOptions {
                file_filter: None,
                section_filter: None,
                has_command: false,
                manual: false,
                tags_expr: None,
                search_expr: None,
                depth: None,
                light: false,
                all: false,
            };
            let had_output = list::run(&opts)?;
            if !had_output {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
