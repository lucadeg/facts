/// The `check` subcommand — run command-facts and report pass/fail/manual.
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::agent;
use crate::color;
use crate::id;
use crate::lint;
use crate::model::FactSheet;
use crate::parser;
use crate::project;
use crate::tags::{matches_search_expr, matches_tag_expr, validate_tag_expr};

/// Options for the check command.
pub struct CheckOptions {
    pub file_filter: Option<String>,
    pub id_filter: Option<Vec<String>>,
    pub section_filter: Option<String>,
    pub tags_expr: Option<String>,
    pub search_expr: Option<String>,
    pub depth: Option<usize>,
    pub timeout: Option<u64>,
    pub agent: Option<String>,
    pub no_agent: bool,
    pub batch_size: Option<usize>,
    pub quiet: bool,
    pub verbose: bool,
}

const DEFAULT_BATCH_SIZE: usize = 10;

/// Result of running a single command-fact.
struct CheckResult {
    id: String,
    display_path: String,
    tags: Vec<String>,
    status: CheckStatus,
}

enum CheckStatus {
    Passed {
        command: String,
    },
    Failed {
        command: String,
        exit_code: i32,
        stderr: String,
    },
    Manual,
    AgentPassed,
    AgentFailed {
        reason: String,
    },
    AgentUnresolved,
}

/// Format the display path for a fact (file > section > label).
fn format_display_path(sheet: &FactSheet, section_path: &[String], label: &str) -> String {
    let file_prefix = sheet.display_name();
    let mut path_parts: Vec<&str> = Vec::new();
    if !file_prefix.is_empty() {
        path_parts.push(file_prefix);
    }
    for s in section_path {
        path_parts.push(s.as_str());
    }

    let dim_sep = color::dim(">");

    if path_parts.is_empty() {
        label.to_string()
    } else {
        let colored_path = path_parts
            .iter()
            .map(|p| color::bold(p))
            .collect::<Vec<_>>()
            .join(&format!(" {dim_sep} "));
        format!("{colored_path} {dim_sep} {label}")
    }
}

/// Format the display path without color (for test assertions).
#[cfg(test)]
fn format_display_path_plain(sheet: &FactSheet, section_path: &[String], label: &str) -> String {
    let file_prefix = sheet.display_name();
    let mut parts: Vec<&str> = Vec::new();
    if !file_prefix.is_empty() {
        parts.push(file_prefix);
    }
    for s in section_path {
        parts.push(s.as_str());
    }
    parts.push(label);
    parts.join(" > ")
}

/// Format tag suffix for display (dimmed @tag @tag).
fn format_tag_suffix(tags: &[String]) -> String {
    if tags.is_empty() {
        String::new()
    } else {
        let tag_str = tags
            .iter()
            .map(|t| format!("@{t}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("  {}", color::dim(&tag_str))
    }
}

/// Execute a command via $SHELL (fallback to sh) and return (exit_code, stderr).
pub(crate) fn run_command(
    command: &str,
    project_root: &Path,
    timeout: Option<Duration>,
) -> (i32, String) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());

    let mut cmd = Command::new(&shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(project_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    match timeout {
        Some(timeout) => run_with_timeout(cmd, timeout),
        None => run_no_timeout(cmd),
    }
}

fn run_no_timeout(mut cmd: Command) -> (i32, String) {
    match cmd.output() {
        Ok(output) => {
            let code = output.status.code().unwrap_or(1);
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (code, stderr)
        }
        Err(e) => (1, format!("failed to run command: {e}")),
    }
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> (i32, String) {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (1, format!("failed to spawn command: {e}")),
    };

    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(50);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut e| {
                        let mut buf = String::new();
                        std::io::Read::read_to_string(&mut e, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();
                return (status.code().unwrap_or(1), stderr);
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (124, "command timed out".to_string());
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                return (1, format!("failed to wait on command: {e}"));
            }
        }
    }
}

fn print_result(out: &mut impl Write, r: &CheckResult, id_width: usize, detail_indent: usize) {
    let padded_id = format!("{:width$}", r.id, width = id_width);
    let tag_suffix = format_tag_suffix(&r.tags);

    match &r.status {
        CheckStatus::Passed { command } => {
            let _ = writeln!(
                out,
                "  {} {} {}{}",
                color::green(&format!("✓ {padded_id}")),
                r.display_path,
                color::dim(&format!("({command})")),
                tag_suffix,
            );
        }
        CheckStatus::Failed {
            command,
            exit_code,
            stderr,
        } => {
            let _ = writeln!(
                out,
                "  {} {}{}",
                color::red(&format!("✗ {padded_id}")),
                r.display_path,
                tag_suffix,
            );
            let pad = " ".repeat(detail_indent);
            let _ = writeln!(out, "{pad}{} {exit_code}", color::dim("exit:"));
            let _ = writeln!(out, "{pad}{} {command}", color::dim("command:"));
            if !stderr.is_empty() {
                for line in stderr.lines() {
                    let _ = writeln!(out, "{pad}{} {line}", color::dim("stderr:"));
                }
            }
        }
        CheckStatus::Manual => {
            let _ = writeln!(
                out,
                "  {} {}{}",
                color::yellow(&format!("? {padded_id}")),
                r.display_path,
                tag_suffix,
            );
        }
        CheckStatus::AgentPassed => {
            let _ = writeln!(
                out,
                "  {} {} {}{}",
                color::green(&format!("✓ {padded_id}")),
                r.display_path,
                color::dim("(agent)"),
                tag_suffix,
            );
        }
        CheckStatus::AgentFailed { reason } => {
            let _ = writeln!(
                out,
                "  {} {}{}",
                color::red(&format!("✗ {padded_id}")),
                r.display_path,
                tag_suffix,
            );
            let pad = " ".repeat(detail_indent);
            let _ = writeln!(out, "{pad}{} agent", color::dim("source:"));
            if !reason.is_empty() {
                let _ = writeln!(out, "{pad}{} {reason}", color::dim("reason:"));
            }
        }
        CheckStatus::AgentUnresolved => {
            let _ = writeln!(
                out,
                "  {} {} {}{}",
                color::yellow(&format!("? {padded_id}")),
                r.display_path,
                color::dim("(unresolved)"),
                tag_suffix,
            );
        }
    }
    let _ = out.flush();
}

/// Run the check subcommand.
pub fn run(opts: &CheckOptions) -> Result<bool> {
    if let Some(ref expr) = opts.tags_expr {
        validate_tag_expr(expr).map_err(|e| anyhow::anyhow!("invalid tag expression: {e}"))?;
    }
    if let Some(ref expr) = opts.search_expr {
        validate_tag_expr(expr).map_err(|e| anyhow::anyhow!("invalid search expression: {e}"))?;
    }

    let root = project::find_project_root()?;
    let files = project::discover_with_explicit(&root, opts.file_filter.as_deref())?;

    if files.is_empty() {
        eprintln!("no .facts files found in {}", root.display());
        return Ok(false);
    }

    // Resolve batch size: CLI flag > .config batch-size > default.
    // Clamp to at least 1 to prevent division-by-zero and infinite loops.
    let batch_size = opts
        .batch_size
        .unwrap_or_else(|| {
            crate::config::read_config_value(&root, "batch-size")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_BATCH_SIZE)
        })
        .max(1);

    // Lint all files first — fail early on structural errors.
    let mut lint_errors = false;
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
                eprintln!(
                    "{}: {}: {}",
                    color::bold(&location),
                    color::red("error"),
                    diag.message
                );
                lint_errors = true;
            }
        }
    }
    if lint_errors {
        eprintln!("\n{}", color::red("check aborted — fix lint errors first"));
        return Ok(false);
    }

    let mut sheets = Vec::new();
    for path in &files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let filename = project::relative_filename(&root, path);
        let sheet = parser::parse(&content, &filename)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        sheets.push(sheet);
    }

    let mut all_fact_labels: Vec<(String, Option<String>)> = Vec::new();
    for sheet in &sheets {
        for (_path, fact) in sheet.all_facts() {
            all_fact_labels.push((fact.label.clone(), fact.explicit_id.clone()));
        }
    }
    let assigned_ids = id::assign_ids(&all_fact_labels);

    let timeout = opts.timeout.map(Duration::from_secs);
    let mut results: Vec<CheckResult> = Vec::new();
    let mut manual_facts: Vec<agent::ManualFact> = Vec::new();
    let mut manual_indices: Vec<usize> = Vec::new();
    let mut fact_idx = 0;

    // Pre-compute ID width for consistent alignment.
    let id_width = assigned_ids.iter().map(|id| id.len()).max().unwrap_or(3);
    let detail_indent = id_width + 6;

    let quiet = opts.quiet;
    let verbose = opts.verbose;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for sheet in &sheets {
        let file_matches = if let Some(ref f) = opts.file_filter {
            sheet.filename == *f || sheet.filename == format!("{f}.facts")
        } else {
            true
        };

        for (section_path, fact) in sheet.all_facts() {
            let id = assigned_ids[fact_idx].clone();
            fact_idx += 1;

            if !file_matches || fact.is_config() {
                continue;
            }

            if let Some(ref ids) = opts.id_filter
                && !ids.iter().any(|f| f == &id)
            {
                continue;
            }

            if let Some(ref section) = opts.section_filter
                && !crate::list::section_matches(&section_path.join("/"), section)
            {
                continue;
            }

            if let Some(ref expr) = opts.tags_expr
                && !matches_tag_expr(expr, &fact.tags)
            {
                continue;
            }

            if let Some(depth) = opts.depth
                && section_path.len() > depth
            {
                continue;
            }

            if let Some(ref expr) = opts.search_expr {
                let haystack = build_search_haystack(&section_path, &fact.label, &fact.tags);
                if !matches_search_expr(expr, &haystack) {
                    continue;
                }
            }

            let display_path = format_display_path(sheet, &section_path, &fact.label);

            let is_tty = color::enabled();

            let status = match &fact.command {
                Some(command) => {
                    if is_tty && !quiet {
                        let padded_id = format!("{:width$}", id, width = id_width);
                        let _ = write!(out, "  {}", color::dim(&format!("… {padded_id}")));
                        let _ = out.flush();
                    }
                    let (exit_code, stderr) = run_command(command, &root, timeout);
                    if is_tty && !quiet {
                        let _ = write!(out, "\r\x1b[2K");
                    }
                    if exit_code == 0 {
                        CheckStatus::Passed {
                            command: command.clone(),
                        }
                    } else {
                        CheckStatus::Failed {
                            command: command.clone(),
                            exit_code,
                            stderr: stderr.trim().to_string(),
                        }
                    }
                }
                None => {
                    manual_facts.push(agent::ManualFact {
                        id: id.clone(),
                        label: fact.label.clone(),
                        section_path: section_path.join("/"),
                    });
                    manual_indices.push(results.len());
                    CheckStatus::Manual
                }
            };

            let result = CheckResult {
                id,
                display_path,
                tags: fact.tags.clone(),
                status,
            };

            if !quiet {
                print_result(&mut out, &result, id_width, detail_indent);
            }
            results.push(result);
        }
    }

    // Agent verification of manual facts.
    let resolved_agent = if opts.no_agent {
        None
    } else {
        agent::resolve_agent(opts.agent.as_deref(), &root)
    };

    if let Some(ref agent) = resolved_agent
        && !manual_facts.is_empty()
    {
        let total_batches = manual_facts.len().div_ceil(batch_size);
        if !quiet {
            let _ = writeln!(
                out,
                "\n{}",
                color::dim(&format!(
                    "Verifying {} manual facts with {} ({} batch{})...",
                    manual_facts.len(),
                    agent.name,
                    total_batches,
                    if total_batches == 1 { "" } else { "es" },
                )),
            );
            let _ = out.flush();
        }

        let is_tty = color::enabled();
        let agent_timeout = timeout;
        for batch_start in (0..manual_facts.len()).step_by(batch_size) {
            let batch_end = (batch_start + batch_size).min(manual_facts.len());
            let batch = &manual_facts[batch_start..batch_end];
            let batch_num = batch_start / batch_size + 1;

            if is_tty && !quiet {
                let _ = write!(
                    out,
                    "  {}",
                    color::dim(&format!("… batch {batch_num}/{total_batches}")),
                );
                let _ = out.flush();
            }

            let batch_result = agent::run_batch(&agent.template, batch, &root, agent_timeout);

            if is_tty && !quiet {
                let _ = write!(out, "\r\x1b[2K");
            }

            // Verbose: show the prompt and raw agent output.
            if verbose && !quiet {
                let _ = writeln!(
                    out,
                    "\n{}",
                    color::dim(&format!("--- batch {batch_num} prompt ---"))
                );
                for line in batch_result.prompt.lines() {
                    let _ = writeln!(out, "  {}", color::dim(line));
                }
                if let Some(ref raw) = batch_result.raw_output {
                    let _ = writeln!(
                        out,
                        "{}",
                        color::dim(&format!("--- batch {batch_num} output ---"))
                    );
                    for line in raw.lines() {
                        let _ = writeln!(out, "  {}", color::dim(line));
                    }
                    let _ = writeln!(out, "{}", color::dim("---"));
                }
            }

            // If the agent process itself failed, show the error once for the batch.
            if !quiet && let Some(ref err) = batch_result.error {
                let _ = writeln!(
                    out,
                    "  {}",
                    color::red(&format!("agent error (batch {batch_num}): {err}")),
                );
            }

            for (i, (_, verdict)) in batch_result.verdicts.iter().enumerate() {
                let result_idx = manual_indices[batch_start + i];
                let status = match verdict {
                    agent::AgentVerdict::Pass => CheckStatus::AgentPassed,
                    agent::AgentVerdict::Fail(reason) => CheckStatus::AgentFailed {
                        reason: reason.clone(),
                    },
                    agent::AgentVerdict::Unresolved => CheckStatus::AgentUnresolved,
                };
                results[result_idx].status = status;
                if !quiet {
                    print_result(&mut out, &results[result_idx], id_width, detail_indent);
                }
            }
        }
    }

    drop(out);

    let passed = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Passed { .. }))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Failed { .. }))
        .count();
    let manual = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Manual))
        .count();
    let agent_passed = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::AgentPassed))
        .count();
    let agent_failed = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::AgentFailed { .. }))
        .count();
    let agent_unresolved = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::AgentUnresolved))
        .count();

    let has_failures = failed > 0 || agent_failed > 0 || agent_unresolved > 0;

    if !quiet {
        let total_passed = passed + agent_passed;
        let total_failed = failed + agent_failed;

        println!();

        // Unified summary line: total passed, total failed, manual/unresolved.
        let mut summary_parts = vec![
            color::green(&format!("{total_passed} passed")),
            color::red(&format!("{total_failed} failed")),
        ];

        if manual > 0 {
            summary_parts.push(color::yellow(&format!("{manual} manual")));
        }

        if agent_unresolved > 0 {
            summary_parts.push(color::yellow(&format!("{agent_unresolved} unresolved")));
        }

        println!("{}", color::bold(&summary_parts.join(", ")));

        // Breakdown line when agent was used: show command vs agent split.
        let agent_total = agent_passed + agent_failed + agent_unresolved;
        if agent_total > 0 {
            let breakdown_parts = [format!("{passed} command"), format!("{agent_passed} agent")];
            println!(
                "{}",
                color::dim(&format!("  passed: {}", breakdown_parts.join(", ")))
            );
            if total_failed > 0 {
                let fail_parts = [format!("{failed} command"), format!("{agent_failed} agent")];
                println!(
                    "{}",
                    color::dim(&format!("  failed: {}", fail_parts.join(", ")))
                );
            }
        }

        if manual > 0 {
            let noun = if manual == 1 {
                "fact has"
            } else {
                "facts have"
            };
            println!(
                "{}",
                color::yellow(&format!(
                    "{manual} {noun} no command — verify each against the code and state PASS or FAIL with a one-line reason."
                )),
            );
            if resolved_agent.is_none() {
                println!(
                    "{}",
                    color::dim(
                        "Use --agent to enable agent verification (e.g. facts check --agent claude)."
                    ),
                );
            }
        }
    }

    Ok(!has_failures)
}

fn build_search_haystack(section_path: &[String], label: &str, tags: &[String]) -> String {
    let mut parts: Vec<&str> = section_path.iter().map(|s| s.as_str()).collect();
    parts.push(label);
    for tag in tags {
        parts.push(tag.as_str());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project_root() -> PathBuf {
        project::find_project_root().unwrap()
    }

    #[test]
    fn test_run_command_success() {
        let (code, _stderr) = run_command("true", &project_root(), None);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_command_failure() {
        let (code, _stderr) = run_command("false", &project_root(), None);
        assert_ne!(code, 0);
    }

    #[test]
    fn test_run_command_captures_stderr() {
        let (code, stderr) = run_command("echo 'oops' >&2; exit 1", &project_root(), None);
        assert_ne!(code, 0);
        assert!(stderr.contains("oops"));
    }

    #[test]
    fn test_run_command_exit_code() {
        let (code, _stderr) = run_command("exit 42", &project_root(), None);
        assert_eq!(code, 42);
    }

    #[test]
    fn test_run_command_uses_project_root_as_cwd() {
        let root = project_root();
        let (code, _stderr) = run_command("test -f Cargo.toml", &root, None);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_command_timeout() {
        let (code, stderr) = run_command(
            "sleep 10",
            &project_root(),
            Some(Duration::from_millis(100)),
        );
        assert_eq!(code, 124);
        assert!(stderr.contains("timed out"));
    }

    #[test]
    fn test_format_display_path_default_file() {
        let sheet = FactSheet {
            filename: ".facts".to_string(),
            preamble: vec![],
            sections: vec![],
        };
        let path = vec!["cli".to_string(), "check".to_string()];
        let result = format_display_path_plain(&sheet, &path, "runs commands");
        assert_eq!(result, "cli > check > runs commands");
    }

    #[test]
    fn test_format_display_path_named_file() {
        let sheet = FactSheet {
            filename: "cli.facts".to_string(),
            preamble: vec![],
            sections: vec![],
        };
        let path = vec!["check".to_string()];
        let result = format_display_path_plain(&sheet, &path, "runs commands");
        assert_eq!(result, "cli.facts > check > runs commands");
    }

    #[test]
    fn test_format_display_path_no_section() {
        let sheet = FactSheet {
            filename: ".facts".to_string(),
            preamble: vec![],
            sections: vec![],
        };
        let result = format_display_path_plain(&sheet, &[], "a preamble fact");
        assert_eq!(result, "a preamble fact");
    }
}
