/// Agent verification for manual facts.
///
/// Supports sending manual facts to an AI agent for verification.
/// The agent receives a prompt with fact IDs and labels, and returns
/// verdicts in XML format: `<fact-verdict id="ID">PASS</fact-verdict>`
/// or `<fact-verdict id="ID">FAIL reason</fact-verdict>`.
use std::path::Path;
use std::process::Command;

/// A preset agent command template.
struct Preset {
    name: &'static str,
    template: &'static str,
}

const PRESETS: &[Preset] = &[
    Preset {
        name: "claude",
        template: "claude -p %p",
    },
    Preset {
        name: "codex",
        template: "codex exec %p",
    },
    Preset {
        name: "pi",
        template: "pi -p %p",
    },
    Preset {
        name: "droid",
        template: "droid exec %p",
    },
    Preset {
        name: "opencode",
        template: "opencode run %p",
    },
    Preset {
        name: "mi",
        template: "mi -p %p",
    },
];

/// A manual fact to be verified by an agent.
pub struct ManualFact {
    pub id: String,
    pub label: String,
    pub section_path: String,
}

/// Result of agent verification for a single fact.
#[derive(Debug, Clone)]
pub enum AgentVerdict {
    Pass,
    Fail(String),
    Unresolved,
}

/// Result of running an agent batch.
pub struct AgentBatchResult {
    pub verdicts: Vec<(String, AgentVerdict)>,
    /// If the agent process itself failed, contains a description of what went wrong.
    pub error: Option<String>,
    /// Raw agent output (stdout + stderr combined), available for verbose display.
    pub raw_output: Option<String>,
    /// The prompt that was sent to the agent.
    pub prompt: String,
}

/// A resolved agent with its command template and display name.
pub struct ResolvedAgent {
    pub template: String,
    pub name: String,
}

/// Resolve which agent command to use.
/// Priority: explicit --agent flag > .config agent key > PATH auto-detect.
pub fn resolve_agent(flag: Option<&str>, project_root: &Path) -> Option<ResolvedAgent> {
    if let Some(agent) = flag {
        let (template, name) = resolve_preset_or_literal(agent);
        return Some(ResolvedAgent { template, name });
    }

    if let Some(config_val) = crate::config::read_config_value(project_root, "agent") {
        let (template, name) = resolve_preset_or_literal(&config_val);
        return Some(ResolvedAgent { template, name });
    }

    auto_detect_agent()
}

fn resolve_preset_or_literal(agent: &str) -> (String, String) {
    for preset in PRESETS {
        if preset.name == agent {
            return (preset.template.to_string(), preset.name.to_string());
        }
    }
    // Extract just the binary name for display (first whitespace-delimited token).
    let name = agent.split_whitespace().next().unwrap_or(agent).to_string();
    (agent.to_string(), name)
}

fn auto_detect_agent() -> Option<ResolvedAgent> {
    for preset in PRESETS {
        if which_exists(preset.name) {
            return Some(ResolvedAgent {
                template: preset.template.to_string(),
                name: preset.name.to_string(),
            });
        }
    }
    None
}

/// Check whether a binary exists on PATH without shelling out to `which`.
fn which_exists(name: &str) -> bool {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let separator = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(separator) {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            // On Unix, also check the executable bit.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = candidate.metadata()
                    && meta.permissions().mode() & 0o111 != 0
                {
                    return true;
                }
                continue;
            }
            #[cfg(not(unix))]
            {
                return true;
            }
        }
    }
    false
}

/// Build the prompt for an agent batch.
fn build_prompt(facts: &[ManualFact]) -> String {
    let mut lines = vec![
        "You are verifying facts about a codebase. Each fact below is an atomic truth claim. Read the relevant source files, configs, and project state to determine whether each fact holds.".to_string(),
        String::new(),
        "For EVERY fact, respond with exactly one XML verdict tag. Do not skip any.".to_string(),
        "- True:  <fact-verdict id=\"ID\">PASS</fact-verdict>".to_string(),
        "- False: <fact-verdict id=\"ID\">FAIL one-line reason</fact-verdict>".to_string(),
        String::new(),
        format!("Facts to verify ({}):", facts.len()),
        String::new(),
    ];

    for fact in facts {
        let path = if fact.section_path.is_empty() {
            fact.label.clone()
        } else {
            format!("{} > {}", fact.section_path, fact.label)
        };
        lines.push(format!("- [{}] {}", fact.id, path));
    }

    lines.join("\n")
}

/// Shell-escape a string for safe use inside a `sh -c` command.
///
/// Uses single-quote wrapping (the strongest POSIX shell quoting):
/// - The entire string is enclosed in single quotes
/// - Any literal single quote becomes `'\''` (end quote, escaped quote, reopen quote)
/// - Control characters (null bytes, etc.) are stripped since they can truncate
///   or corrupt shell arguments
fn shell_escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            escaped.push_str("'\\''");
        } else if ch == '\0' {
            // Null bytes can truncate C-string arguments — skip them.
            continue;
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

/// Run an agent batch and return verdicts.
pub fn run_batch(
    template: &str,
    facts: &[ManualFact],
    project_root: &Path,
    timeout: Option<std::time::Duration>,
) -> AgentBatchResult {
    if facts.is_empty() {
        return AgentBatchResult {
            verdicts: Vec::new(),
            error: None,
            raw_output: None,
            prompt: String::new(),
        };
    }

    let prompt = build_prompt(facts);
    let escaped = shell_escape(&prompt);
    let command = if template.contains("%p") {
        template.replace("%p", &escaped)
    } else {
        // Template has no prompt placeholder — append the escaped prompt as an argument.
        format!("{template} {escaped}")
    };

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());

    let mut cmd = Command::new(&shell);
    cmd.arg("-c")
        .arg(&command)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let result = match timeout {
        Some(dur) => run_agent_with_timeout(cmd, dur),
        None => run_agent_no_timeout(cmd),
    };

    match result {
        Ok((stdout, stderr)) => {
            let combined = format!("{stdout}{stderr}");
            let mut batch_result = parse_verdicts(&combined, facts);
            batch_result.raw_output = Some(combined);
            batch_result.prompt = prompt;
            batch_result
        }
        Err(error_msg) => {
            let verdicts = facts
                .iter()
                .map(|f| (f.id.clone(), AgentVerdict::Unresolved))
                .collect();
            AgentBatchResult {
                verdicts,
                error: Some(error_msg),
                raw_output: None,
                prompt,
            }
        }
    }
}

/// Format an error message for a failed agent process, including exit code
/// and the first line of stderr as a hint when available.
fn format_exit_error(code: i32, stderr: &str) -> String {
    let hint = stderr
        .trim()
        .lines()
        .next()
        .filter(|l| !l.is_empty())
        .map(|l| format!(": {l}"))
        .unwrap_or_default();
    format!("agent exited with code {code}{hint}")
}

/// Run agent command without timeout, returning (stdout, stderr) or an error message.
fn run_agent_no_timeout(mut cmd: Command) -> Result<(String, String), String> {
    match cmd.output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok((stdout, stderr))
        }
        Ok(output) => {
            let code = output.status.code().unwrap_or(1);
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(format_exit_error(code, &stderr))
        }
        Err(e) => Err(format!("failed to run agent: {e}")),
    }
}

/// Read all content from an optional pipe, returning empty string if absent.
fn drain_pipe(pipe: Option<impl std::io::Read>) -> String {
    pipe.map(|mut r| {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut r, &mut buf).ok();
        buf
    })
    .unwrap_or_default()
}

/// Run agent command with timeout, returning (stdout, stderr) or an error message.
fn run_agent_with_timeout(
    mut cmd: Command,
    timeout: std::time::Duration,
) -> Result<(String, String), String> {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Err(format!("failed to start agent: {e}")),
    };

    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = drain_pipe(child.stdout.take());
                let stderr = drain_pipe(child.stderr.take());
                if status.success() {
                    return Ok((stdout, stderr));
                }
                return Err(format_exit_error(status.code().unwrap_or(1), &stderr));
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("agent timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                return Err(format!("failed to wait on agent: {e}"));
            }
        }
    }
}

/// Parse agent output for fact-verdict tags.
fn parse_verdicts(output: &str, facts: &[ManualFact]) -> AgentBatchResult {
    let cleaned = strip_ansi_and_fences(output);
    let mut verdict_map: std::collections::HashMap<String, AgentVerdict> =
        std::collections::HashMap::new();

    for cap in find_fact_verdicts(&cleaned) {
        verdict_map.insert(cap.id, cap.verdict);
    }

    let verdicts = facts
        .iter()
        .map(|f| {
            let verdict = verdict_map
                .remove(&f.id)
                .unwrap_or(AgentVerdict::Unresolved);
            (f.id.clone(), verdict)
        })
        .collect();

    AgentBatchResult {
        verdicts,
        error: None,
        raw_output: None,
        prompt: String::new(),
    }
}

struct ParsedVerdict {
    id: String,
    verdict: AgentVerdict,
}

fn find_fact_verdicts(text: &str) -> Vec<ParsedVerdict> {
    let mut results = Vec::new();
    let open_tag = "<fact-verdict";
    let close_tag = "</fact-verdict>";

    let mut search_from = 0;
    while let Some(start) = text[search_from..].find(open_tag) {
        let abs_start = search_from + start;
        let after_tag = abs_start + open_tag.len();

        let Some(gt_pos) = text[after_tag..].find('>') else {
            break;
        };
        let attrs = &text[after_tag..after_tag + gt_pos];
        let content_start = after_tag + gt_pos + 1;

        let Some(end) = text[content_start..].find(close_tag) else {
            break;
        };
        let content = text[content_start..content_start + end].trim();

        if let Some(id) = extract_id_attr(attrs) {
            let verdict = if content.eq_ignore_ascii_case("pass") {
                AgentVerdict::Pass
            } else if let Some(reason) = content
                .strip_prefix("FAIL")
                .or_else(|| content.strip_prefix("fail"))
                .or_else(|| content.strip_prefix("Fail"))
            {
                AgentVerdict::Fail(reason.trim().to_string())
            } else {
                AgentVerdict::Fail(content.to_string())
            };
            results.push(ParsedVerdict { id, verdict });
        }

        search_from = content_start + end + close_tag.len();
    }

    results
}

fn extract_id_attr(attrs: &str) -> Option<String> {
    let id_prefix = "id=\"";
    if let Some(start) = attrs.find(id_prefix) {
        let val_start = start + id_prefix.len();
        if let Some(end) = attrs[val_start..].find('"') {
            return Some(attrs[val_start..val_start + end].to_string());
        }
    }
    let id_prefix_single = "id='";
    if let Some(start) = attrs.find(id_prefix_single) {
        let val_start = start + id_prefix_single.len();
        if let Some(end) = attrs[val_start..].find('\'') {
            return Some(attrs[val_start..val_start + end].to_string());
        }
    }
    None
}

/// Strip ANSI escape codes and markdown fences from text.
fn strip_ansi_and_fences(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            while let Some(&c) = chars.peek() {
                chars.next();
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        result.push(ch);
    }

    let mut final_result = String::new();
    for line in result.lines() {
        if line.trim().starts_with("```") {
            continue;
        }
        final_result.push_str(line);
        final_result.push('\n');
    }

    final_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_preset() {
        assert_eq!(
            resolve_preset_or_literal("claude"),
            ("claude -p %p".to_string(), "claude".to_string())
        );
        assert_eq!(
            resolve_preset_or_literal("codex"),
            ("codex exec %p".to_string(), "codex".to_string())
        );
        assert_eq!(
            resolve_preset_or_literal("custom cmd %p"),
            ("custom cmd %p".to_string(), "custom".to_string())
        );
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape("a b c"), "'a b c'");
    }

    #[test]
    fn test_parse_verdicts_pass() {
        let output = r#"<fact-verdict id="abc">PASS</fact-verdict>"#;
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let result = parse_verdicts(output, &facts);
        assert!(matches!(result.verdicts[0].1, AgentVerdict::Pass));
    }

    #[test]
    fn test_parse_verdicts_fail() {
        let output = r#"<fact-verdict id="abc">FAIL not implemented yet</fact-verdict>"#;
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let result = parse_verdicts(output, &facts);
        assert!(
            matches!(result.verdicts[0].1, AgentVerdict::Fail(ref r) if r == "not implemented yet")
        );
    }

    #[test]
    fn test_parse_verdicts_unresolved() {
        let output = "no verdict tags here";
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let result = parse_verdicts(output, &facts);
        assert!(matches!(result.verdicts[0].1, AgentVerdict::Unresolved));
    }

    #[test]
    fn test_parse_verdicts_last_wins() {
        let output = r#"
<fact-verdict id="abc">FAIL first</fact-verdict>
<fact-verdict id="abc">PASS</fact-verdict>
"#;
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let result = parse_verdicts(output, &facts);
        assert!(matches!(result.verdicts[0].1, AgentVerdict::Pass));
    }

    #[test]
    fn test_strip_ansi_codes() {
        let input = "\x1b[32mgreen\x1b[0m normal";
        let result = strip_ansi_and_fences(input);
        assert_eq!(result.trim(), "green normal");
    }

    #[test]
    fn test_strip_markdown_fences() {
        let input = "```xml\n<fact-verdict id=\"x\">PASS</fact-verdict>\n```\n";
        let result = strip_ansi_and_fences(input);
        assert!(result.contains("<fact-verdict"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn test_build_prompt_includes_facts() {
        let facts = vec![
            ManualFact {
                id: "abc".to_string(),
                label: "some fact".to_string(),
                section_path: "cli/check".to_string(),
            },
            ManualFact {
                id: "def".to_string(),
                label: "another fact".to_string(),
                section_path: String::new(),
            },
        ];
        let prompt = build_prompt(&facts);
        assert!(prompt.contains("[abc]"));
        assert!(prompt.contains("cli/check > some fact"));
        assert!(prompt.contains("[def]"));
        assert!(prompt.contains("another fact"));
        assert!(prompt.contains("fact-verdict"));
    }

    #[test]
    fn test_build_prompt_includes_fact_count() {
        let facts = vec![
            ManualFact {
                id: "a".to_string(),
                label: "one".to_string(),
                section_path: String::new(),
            },
            ManualFact {
                id: "b".to_string(),
                label: "two".to_string(),
                section_path: String::new(),
            },
            ManualFact {
                id: "c".to_string(),
                label: "three".to_string(),
                section_path: String::new(),
            },
        ];
        let prompt = build_prompt(&facts);
        assert!(
            prompt.contains("Facts to verify (3):"),
            "prompt should include fact count: {prompt}"
        );
    }

    #[test]
    fn test_build_prompt_instructs_no_skipping() {
        let facts = vec![ManualFact {
            id: "x".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let prompt = build_prompt(&facts);
        assert!(
            prompt.contains("Do not skip any"),
            "prompt should instruct agent not to skip facts: {prompt}"
        );
    }

    #[test]
    fn test_custom_template_name_extracts_binary() {
        // A literal template should use the first word (binary name) as display name.
        let (template, name) = resolve_preset_or_literal("/usr/local/bin/my-agent -p %p --verbose");
        assert_eq!(template, "/usr/local/bin/my-agent -p %p --verbose");
        assert_eq!(name, "/usr/local/bin/my-agent");
    }

    #[test]
    fn test_preset_name_unchanged() {
        let (_, name) = resolve_preset_or_literal("claude");
        assert_eq!(name, "claude");
    }

    #[test]
    fn test_multiple_facts_in_output() {
        let output = r#"
<fact-verdict id="a1">PASS</fact-verdict>
<fact-verdict id="b2">FAIL missing feature</fact-verdict>
<fact-verdict id="c3">PASS</fact-verdict>
"#;
        let facts = vec![
            ManualFact {
                id: "a1".to_string(),
                label: "one".to_string(),
                section_path: String::new(),
            },
            ManualFact {
                id: "b2".to_string(),
                label: "two".to_string(),
                section_path: String::new(),
            },
            ManualFact {
                id: "c3".to_string(),
                label: "three".to_string(),
                section_path: String::new(),
            },
        ];
        let result = parse_verdicts(output, &facts);
        assert!(matches!(result.verdicts[0].1, AgentVerdict::Pass));
        assert!(
            matches!(result.verdicts[1].1, AgentVerdict::Fail(ref r) if r == "missing feature")
        );
        assert!(matches!(result.verdicts[2].1, AgentVerdict::Pass));
    }

    #[test]
    fn test_ansi_around_verdicts() {
        let output =
            "\x1b[1m\x1b[32m<fact-verdict id=\"x\">\x1b[0mPASS\x1b[32m</fact-verdict>\x1b[0m";
        let facts = vec![ManualFact {
            id: "x".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let result = parse_verdicts(output, &facts);
        assert!(matches!(result.verdicts[0].1, AgentVerdict::Pass));
    }

    // =========================================================================
    // Security and robustness tests
    // =========================================================================

    #[test]
    fn test_shell_escape_backticks() {
        // Backticks inside single quotes are literal — no command substitution.
        let result = shell_escape("hello `whoami` world");
        assert_eq!(result, "'hello `whoami` world'");
    }

    #[test]
    fn test_shell_escape_dollar_expansion() {
        // Dollar signs inside single quotes are literal — no variable expansion.
        let result = shell_escape("the value is $HOME");
        assert_eq!(result, "'the value is $HOME'");
    }

    #[test]
    fn test_shell_escape_double_quotes() {
        let result = shell_escape(r#"say "hello""#);
        assert_eq!(result, r#"'say "hello"'"#);
    }

    #[test]
    fn test_shell_escape_newlines() {
        // Newlines inside single quotes are literal.
        let result = shell_escape("line1\nline2");
        assert_eq!(result, "'line1\nline2'");
    }

    #[test]
    fn test_shell_escape_null_bytes_stripped() {
        // Null bytes are stripped to prevent C-string truncation.
        let result = shell_escape("before\0after");
        assert_eq!(result, "'beforeafter'");
    }

    #[test]
    fn test_shell_escape_semicolons_and_pipes() {
        // Semicolons and pipes inside single quotes are literal.
        let result = shell_escape("true; rm -rf / | cat");
        assert_eq!(result, "'true; rm -rf / | cat'");
    }

    #[test]
    fn test_shell_escape_complex_injection_attempt() {
        // An adversarial fact label trying to break out of single quotes.
        // Verify that shell_escape produces a string that, when evaluated by sh,
        // yields the original input as a single argument (no injection).
        let input = "fact'; rm -rf /; echo '";
        let escaped = shell_escape(input);
        // Must start and end with single quotes.
        assert!(
            escaped.starts_with('\''),
            "must start with single quote: {escaped}"
        );
        assert!(
            escaped.ends_with('\''),
            "must end with single quote: {escaped}"
        );
        // Integration check: pass through sh -c printf and verify the output matches input.
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {escaped}"))
            .output()
            .expect("failed to run sh");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout, input,
            "shell round-trip of escaped string should recover original"
        );
    }

    #[test]
    fn test_shell_escape_unicode() {
        let result = shell_escape("emoji \u{1F600} and CJK \u{4E16}\u{754C}");
        assert_eq!(result, "'emoji \u{1F600} and CJK \u{4E16}\u{754C}'");
    }

    #[test]
    fn test_shell_escape_empty_string() {
        let result = shell_escape("");
        assert_eq!(result, "''");
    }

    #[test]
    fn test_build_prompt_unicode_labels() {
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "supports unicode \u{1F680} labels".to_string(),
            section_path: String::new(),
        }];
        let prompt = build_prompt(&facts);
        assert!(
            prompt.contains("\u{1F680}"),
            "unicode should survive prompt building"
        );
    }

    #[test]
    fn test_build_prompt_special_chars_in_label() {
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "fact with `backticks` and $vars and \"quotes\"".to_string(),
            section_path: String::new(),
        }];
        let prompt = build_prompt(&facts);
        assert!(
            prompt.contains("`backticks`"),
            "backticks should be preserved in prompt"
        );
        assert!(
            prompt.contains("$vars"),
            "dollar signs should be preserved in prompt"
        );
        assert!(
            prompt.contains("\"quotes\""),
            "double quotes should be preserved in prompt"
        );
    }

    #[test]
    fn test_parse_verdicts_unicode_in_reason() {
        let output = "<fact-verdict id=\"abc\">FAIL missing \u{1F4A5} feature</fact-verdict>";
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "test".to_string(),
            section_path: String::new(),
        }];
        let result = parse_verdicts(output, &facts);
        match &result.verdicts[0].1 {
            AgentVerdict::Fail(reason) => assert!(
                reason.contains("\u{1F4A5}"),
                "unicode should survive verdict parsing: {reason}"
            ),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn test_run_batch_empty_facts() {
        // Empty batch should return immediately, not invoke the agent.
        let result = run_batch("nonexistent-agent %p", &[], Path::new("."), None);
        assert!(result.verdicts.is_empty());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_which_exists_detects_sh() {
        // /bin/sh should always exist on Unix-like systems.
        assert!(which_exists("sh"), "sh should be found on PATH");
    }

    #[test]
    fn test_which_exists_rejects_nonexistent() {
        assert!(
            !which_exists("definitely_not_a_real_binary_xyz_123"),
            "nonexistent binary should not be found"
        );
    }

    #[test]
    fn test_template_without_percent_p_appends_prompt() {
        // When template has no %p, the escaped prompt should be appended.
        // We test this indirectly by running a simple echo command.
        let facts = vec![ManualFact {
            id: "abc".to_string(),
            label: "test fact".to_string(),
            section_path: String::new(),
        }];
        // Use "echo" as template (no %p). The prompt is appended as argument.
        let result = run_batch("echo", &facts, Path::new("."), None);
        // The raw output should contain some text (echo printed the prompt).
        assert!(
            result.raw_output.is_some(),
            "echo command should produce output"
        );
        assert!(
            result.error.is_none(),
            "echo command should not error: {:?}",
            result.error
        );
    }
}
