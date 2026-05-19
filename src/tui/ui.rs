use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use unicode_width::UnicodeWidthStr;

use super::form::{FormMode, FormState};
use super::theme;
use super::{App, CheckResult, Mode};

fn display_width_before(s: &str, byte_index: usize) -> usize {
    s[..byte_index.min(s.len())].width()
}

fn filter_prefix_width(app: &App) -> usize {
    let search_w = app.search.width();
    match app.filter_field {
        0 => 2,                // " /"
        1 => 2 + search_w + 3, // " /" + search + "  @"
        2 => {
            let tags_w = if app.tags_filter.is_empty() {
                0
            } else {
                3 + app.tags_filter.width() // "  @" + tags text
            };
            2 + search_w + tags_w + 3 // " /" + search + [tags] + "  #"
        }
        _ => 2,
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let main_block = Block::default()
        .title(Line::from(vec![Span::styled(
            " FACTS ",
            Style::default()
                .fg(theme::PURPLE)
                .add_modifier(Modifier::BOLD),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG));

    let inner = main_block.inner(area);
    frame.render_widget(main_block, area);

    let content_area = if app.no_git {
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(inner);
        let warning = Paragraph::new(Line::from(vec![
            Span::styled(
                " WARNING ",
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::MANUAL)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " no project root — operating in cwd",
                Style::default().fg(theme::MANUAL),
            ),
        ]))
        .style(Style::default().bg(theme::SURFACE));
        frame.render_widget(warning, chunks[0]);
        chunks[1]
    } else {
        inner
    };

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(content_area);

    draw_filter_bar(frame, app, chunks[0]);

    if app.show_tree {
        let hsplit =
            Layout::horizontal([Constraint::Length(28), Constraint::Fill(1)]).split(chunks[1]);
        draw_tree(frame, app, hsplit[0]);
        app.table_height = hsplit[1].height.saturating_sub(1) as usize;
        draw_table(frame, app, hsplit[1]);
    } else {
        app.table_height = chunks[1].height.saturating_sub(1) as usize;
        draw_table(frame, app, chunks[1]);
    }

    draw_lifecycle_bar(frame, app, chunks[2]);
    draw_shortcuts(frame, app, chunks[3]);
    draw_status_bar(frame, app, chunks[4]);

    match app.mode {
        Mode::Help => draw_help(frame, area),
        Mode::Detail => draw_detail(frame, app, area),
        Mode::Form => {
            if let Some(ref form) = app.form {
                draw_form(frame, form, area);
            }
        }
        Mode::ConfirmDelete => draw_confirm_delete(frame, app, area),
        _ => {}
    }
}

fn draw_filter_bar(frame: &mut Frame, app: &App, area: Rect) {
    let is_filter = app.mode == Mode::Filter;
    let mut spans = Vec::new();

    let search_style = if is_filter && app.filter_field == 0 {
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD)
    } else if !app.search.is_empty() {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::MUTED)
    };
    spans.push(Span::styled(" /", Style::default().fg(theme::MUTED)));
    let search_text = if app.search.is_empty() && !is_filter {
        "search"
    } else {
        &app.search
    };
    spans.push(Span::styled(search_text, search_style));

    if !app.tags_filter.is_empty() || (is_filter && app.filter_field == 1) {
        spans.push(Span::styled("  @", Style::default().fg(theme::MUTED)));
        let style = if is_filter && app.filter_field == 1 {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::ACCENT)
        };
        let text = if app.tags_filter.is_empty() {
            "tags"
        } else {
            &app.tags_filter
        };
        spans.push(Span::styled(text, style));
    }

    if !app.section_filter.is_empty() || (is_filter && app.filter_field == 2) {
        spans.push(Span::styled("  #", Style::default().fg(theme::MUTED)));
        let style = if is_filter && app.filter_field == 2 {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::ACCENT)
        };
        let text = if app.section_filter.is_empty() {
            "section"
        } else {
            &app.section_filter
        };
        spans.push(Span::styled(text, style));
    }

    if app.manual_only {
        spans.push(Span::styled("  [M]", Style::default().fg(theme::MANUAL)));
    }
    if app.command_only {
        spans.push(Span::styled("  [Cmd]", Style::default().fg(theme::PASS)));
    }

    let count_text = format!("{}/{}", app.filtered_indices.len(), app.display_facts.len());
    let used: usize = spans.iter().map(|s| s.width()).sum();
    let available = (area.width as usize).saturating_sub(used + count_text.len() + 1);
    if available > 0 {
        spans.push(Span::raw(" ".repeat(available)));
    }
    spans.push(Span::styled(count_text, Style::default().fg(theme::MUTED)));

    let bar = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::SURFACE));
    frame.render_widget(bar, area);

    if is_filter {
        let prefix_width = filter_prefix_width(app);
        let field_val = match app.filter_field {
            0 => &app.search,
            1 => &app.tags_filter,
            2 => &app.section_filter,
            _ => &app.search,
        };
        let cursor_w = display_width_before(field_val, app.filter_cursor);
        let cx = area.x + prefix_width as u16 + cursor_w as u16;
        frame.set_cursor_position(Position::new(
            cx.min(area.right().saturating_sub(1)),
            area.y,
        ));
    }
}

fn draw_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let header = Row::new(
        [" ID", "Section", "Label", "Tags", "C", "S"]
            .into_iter()
            .map(|h| {
                Cell::from(h).style(
                    Style::default()
                        .fg(theme::PURPLE)
                        .add_modifier(Modifier::BOLD),
                )
            }),
    )
    .style(Style::default().bg(theme::SURFACE))
    .height(1);

    let rows: Vec<Row> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let fact = &app.display_facts[idx];

            let id_cell =
                Cell::from(format!(" {}", fact.id)).style(Style::default().fg(theme::MUTED));

            let path = fact.display_path();
            let path_cell = Cell::from(path).style(Style::default().fg(theme::TEXT));

            let label_cell = Cell::from(fact.label.clone()).style(Style::default().fg(theme::TEXT));

            let tags_str = fact
                .tags
                .iter()
                .map(|t| format!("@{t}"))
                .collect::<Vec<_>>()
                .join(" ");
            let tags_style = if fact.tags.iter().any(|t| t == "implemented") {
                Style::default().fg(theme::PASS)
            } else if fact.tags.iter().any(|t| t == "spec") {
                Style::default()
                    .fg(theme::HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
            } else if fact.tags.iter().any(|t| t == "draft") {
                Style::default().fg(theme::MANUAL)
            } else {
                Style::default().fg(theme::MUTED)
            };
            let tags_cell = Cell::from(tags_str).style(tags_style);

            let cmd_cell = Cell::from(if fact.has_command { "✓" } else { " " })
                .style(Style::default().fg(theme::MUTED));

            let status_cell = if let Some(result) = app.check_results.get(&fact.global_index) {
                match result {
                    CheckResult::Passed => Cell::from("✓").style(Style::default().fg(theme::PASS)),
                    CheckResult::Failed(_) => {
                        Cell::from("✗").style(Style::default().fg(theme::FAIL))
                    }
                    CheckResult::Manual => {
                        Cell::from("?").style(Style::default().fg(theme::MANUAL))
                    }
                }
            } else if app.check_running {
                Cell::from("…").style(Style::default().fg(theme::MUTED))
            } else {
                Cell::from(" ")
            };

            Row::new(vec![
                id_cell,
                path_cell,
                label_cell,
                tags_cell,
                cmd_cell,
                status_cell,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Percentage(18),
        Constraint::Fill(1),
        Constraint::Percentage(15),
        Constraint::Length(2),
        Constraint::Length(2),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(theme::HIGHLIGHT)
                .fg(theme::BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸")
        .style(Style::default().bg(theme::BG));

    if !app.filtered_indices.is_empty() {
        app.table_state.select(Some(app.selected));
    } else {
        app.table_state.select(None);
    }

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn shortcut_items_for_mode(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.mode {
        Mode::Browse if app.show_tree && app.tree_focused => vec![
            ("↑↓", "nav"),
            ("l", "expand"),
            ("h", "collapse"),
            ("Tab", "table"),
            ("a", "add here"),
            ("t", "hide tree"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Mode::Browse => {
            let mut v = vec![
                ("↑↓", "nav"),
                ("/", "filter"),
                ("Enter", "detail"),
                ("a", "add"),
                ("e", "edit"),
                ("d", "del"),
                ("m", "move"),
                ("x", "check one"),
            ];
            if app.show_tree {
                v.push(("Tab", "tree"));
            }
            v.push(("t", "sections"));
            if app.check_running {
                v.push(("C", "cancel"));
            } else {
                v.push(("c", "check all"));
            }
            v.push(("?", "help"));
            v.push(("q", "quit"));
            v
        }
        Mode::Filter => vec![
            ("Tab", "next field"),
            ("S-Tab", "prev field"),
            ("Ctrl-U", "clear"),
            ("Enter", "done"),
            ("Esc", "cancel"),
        ],
        Mode::Help => vec![("q", "close"), ("Esc", "close")],
        Mode::Detail => vec![
            ("x", "check"),
            ("e", "edit"),
            ("d", "del"),
            ("q", "close"),
            ("Esc", "close"),
        ],
        Mode::ConfirmDelete => vec![("y", "confirm"), ("n", "cancel"), ("Esc", "cancel")],
        Mode::Form => vec![
            ("Tab", "next field"),
            ("S-Tab", "prev field"),
            ("Ctrl-S", "submit"),
            ("Ctrl-U", "clear field"),
            ("Esc", "cancel"),
        ],
    }
}

fn build_shortcut_spans(items: &[(&str, &str)], max_width: usize) -> Vec<Span<'static>> {
    fn total_width(spans: &[Span]) -> usize {
        spans.iter().map(|s| s.width()).sum()
    }

    fn make_spans(items: &[(&str, &str)], keys_only: bool) -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled(" ", Style::default())];
        for (key, label) in items {
            spans.push(Span::styled(
                format!(" {key}"),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
            if !keys_only {
                spans.push(Span::styled(
                    format!(" {label}"),
                    Style::default().fg(theme::TEXT),
                ));
            }
        }
        spans
    }

    fn truncate_spans(spans: &mut Vec<Span<'static>>, max_width: usize) {
        while spans.len() > 1 {
            let w = total_width(spans);
            if w <= max_width {
                break;
            }
            spans.pop();
        }
        if total_width(spans) > max_width && spans.len() > 1 {
            spans.truncate(1);
        }
        if spans.len() > 1 && total_width(spans) + 2 <= max_width {
            spans.push(Span::styled(" …", Style::default().fg(theme::MUTED)));
        }
    }

    let mut spans = make_spans(items, false);
    if total_width(&spans) > max_width {
        spans = make_spans(items, true);
    }
    if total_width(&spans) > max_width {
        truncate_spans(&mut spans, max_width);
    }
    spans
}

fn draw_shortcuts(frame: &mut Frame, app: &App, area: Rect) {
    let items = shortcut_items_for_mode(app);
    let spans = build_shortcut_spans(&items, area.width as usize);
    let bar = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::BG));
    frame.render_widget(bar, area);
}

fn draw_tree(frame: &mut Frame, app: &App, area: Rect) {
    use super::tree;

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = tree::visible_indices(&app.tree_nodes);

    let header = Line::from(Span::styled(
        " SECTIONS",
        Style::default()
            .fg(theme::PURPLE)
            .add_modifier(Modifier::BOLD),
    ));

    let viewport_height = inner.height.saturating_sub(1) as usize;
    let scroll_offset = if app.tree_selected >= viewport_height {
        app.tree_selected - viewport_height + 1
    } else {
        0
    };

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().bg(theme::BG)),
        header_area,
    );

    let tree_area = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );

    let w = tree_area.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (vi, &ni) in visible.iter().enumerate() {
        let node = &app.tree_nodes[ni];
        let indent = "  ".repeat(node.depth);
        let arrow = if node.has_children {
            if node.expanded { "▾ " } else { "▸ " }
        } else {
            "  "
        };
        let count_str = if node.fact_count > 0 {
            format!(" {}", node.fact_count)
        } else {
            String::new()
        };

        let is_selected = vi == app.tree_selected;
        let content = format!(" {indent}{arrow}{}{count_str}", node.title);
        let padded = format!("{content:<w$}");

        let style = if is_selected && app.tree_focused {
            Style::default()
                .fg(theme::BG)
                .bg(theme::HIGHLIGHT)
                .add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default().fg(theme::TEXT).bg(theme::SURFACE)
        } else {
            Style::default().fg(theme::TEXT)
        };

        lines.push(Line::from(Span::styled(padded, style)));
    }

    let tree_widget = Paragraph::new(lines)
        .style(Style::default().bg(theme::BG))
        .scroll((scroll_offset as u16, 0));
    frame.render_widget(tree_widget, tree_area);
}

fn draw_lifecycle_bar(frame: &mut Frame, app: &App, area: Rect) {
    let (imp, spec, draft, other) = app.lifecycle_counts();
    let total = imp + spec + draft + other;
    if total == 0 {
        let empty = Paragraph::new("").style(Style::default().bg(theme::BG));
        frame.render_widget(empty, area);
        return;
    }

    let bar_width = 20usize.min(area.width as usize / 3);
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", Style::default()));

    let blocks = |count: usize| -> usize {
        if count == 0 {
            0
        } else {
            ((count * bar_width) / total).max(1)
        }
    };

    let imp_b = blocks(imp);
    let spec_b = blocks(spec);
    let draft_b = blocks(draft);
    let other_b = bar_width.saturating_sub(imp_b + spec_b + draft_b);

    if imp_b > 0 {
        spans.push(Span::styled(
            "█".repeat(imp_b),
            Style::default().fg(theme::PASS),
        ));
    }
    if spec_b > 0 {
        spans.push(Span::styled(
            "█".repeat(spec_b),
            Style::default().fg(theme::HIGHLIGHT),
        ));
    }
    if draft_b > 0 {
        spans.push(Span::styled(
            "█".repeat(draft_b),
            Style::default().fg(theme::MANUAL),
        ));
    }
    if other_b > 0 {
        spans.push(Span::styled(
            "░".repeat(other_b),
            Style::default().fg(theme::MUTED),
        ));
    }

    spans.push(Span::styled("  ", Style::default()));

    if imp > 0 {
        spans.push(Span::styled(
            format!("{imp}"),
            Style::default().fg(theme::PASS),
        ));
        spans.push(Span::styled(" done  ", Style::default().fg(theme::MUTED)));
    }
    if spec > 0 {
        spans.push(Span::styled(
            format!("{spec}"),
            Style::default().fg(theme::HIGHLIGHT),
        ));
        spans.push(Span::styled(" spec  ", Style::default().fg(theme::MUTED)));
    }
    if draft > 0 {
        spans.push(Span::styled(
            format!("{draft}"),
            Style::default().fg(theme::MANUAL),
        ));
        spans.push(Span::styled(" draft  ", Style::default().fg(theme::MUTED)));
    }
    if other > 0 {
        spans.push(Span::styled(
            format!("{other}"),
            Style::default().fg(theme::MUTED),
        ));
        spans.push(Span::styled(" ─", Style::default().fg(theme::MUTED)));
    }

    let bar = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::BG));
    frame.render_widget(bar, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();

    let mode_name = match app.mode {
        Mode::Browse => "BROWSE",
        Mode::Filter => "FILTER",
        Mode::Help => "HELP",
        Mode::Form => match app.form.as_ref().map(|f| &f.mode) {
            Some(FormMode::Add) => "ADD",
            Some(FormMode::Edit) => "EDIT",
            Some(FormMode::Move) => "MOVE",
            None => "FORM",
        },
        Mode::Detail => "DETAIL",
        Mode::ConfirmDelete => "DELETE?",
    };

    spans.push(Span::styled(
        format!(" {mode_name} "),
        Style::default()
            .fg(theme::BG)
            .bg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    ));

    if let Some(fact) = app.selected_fact() {
        spans.push(Span::styled(
            format!(" {} ", fact.id),
            Style::default().fg(theme::TEXT),
        ));
    }

    let root_display = app.project_root.display().to_string();
    let root_short = if root_display.len() > 30 {
        format!("…{}", &root_display[root_display.len() - 28..])
    } else {
        root_display
    };
    spans.push(Span::styled(
        format!(" {root_short} "),
        Style::default().fg(theme::MUTED),
    ));

    if app.check_running {
        let total = app.check_passed + app.check_failed + app.check_manual;
        spans.push(Span::styled(
            format!(" checking {}/{} ", total, app.display_facts.len()),
            Style::default().fg(theme::MANUAL),
        ));
    } else if app.check_passed + app.check_failed + app.check_manual > 0 {
        spans.push(Span::styled(
            format!(" {}✓", app.check_passed),
            Style::default().fg(theme::PASS),
        ));
        spans.push(Span::styled(
            format!(" {}✗", app.check_failed),
            Style::default().fg(theme::FAIL),
        ));
        spans.push(Span::styled(
            format!(" {}?", app.check_manual),
            Style::default().fg(theme::MANUAL),
        ));
    }

    if !app.status.is_empty() {
        let used: usize = spans.iter().map(|s| s.width()).sum();
        let pad = (area.width as usize).saturating_sub(used + app.status.len() + 1);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        spans.push(Span::styled(
            app.status.clone(),
            Style::default().fg(theme::MUTED),
        ));
    }

    let bar = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::SURFACE));
    frame.render_widget(bar, area);
}

fn draw_confirm_delete(frame: &mut Frame, app: &App, area: Rect) {
    let confirm_area = centered_rect(50, 20, area);
    frame.render_widget(Clear, confirm_area);

    let block = Block::default()
        .title(Line::from(Span::styled(
            " DELETE FACT ",
            Style::default()
                .fg(theme::FAIL)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::FAIL))
        .style(Style::default().bg(theme::BG));

    let inner = block.inner(confirm_area);
    frame.render_widget(block, confirm_area);

    let id = app.pending_delete.as_deref().unwrap_or("?");
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Delete fact ", Style::default().fg(theme::TEXT)),
            Span::styled(id.to_string(), Style::default().fg(theme::ACCENT)),
            Span::styled("?", Style::default().fg(theme::TEXT)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default().fg(theme::TEXT)),
            Span::styled(
                "y",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" confirm   ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "n",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel", Style::default().fg(theme::MUTED)),
        ]),
    ];

    let dialog = Paragraph::new(lines).style(Style::default().bg(theme::BG));
    frame.render_widget(dialog, inner);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let help_area = centered_rect(60, 70, area);
    frame.render_widget(Clear, help_area);

    let block = Block::default()
        .title(Line::from(Span::styled(
            " KEYBINDINGS ",
            Style::default()
                .fg(theme::PURPLE)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG));

    let inner = block.inner(help_area);
    frame.render_widget(block, help_area);

    let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).split(inner);

    let bindings = [
        ("j/k ↑↓", "Navigate"),
        ("G / End", "Bottom"),
        ("Home", "Top"),
        ("PgDn/Up", "Page scroll"),
        ("/", "Filter (search, tags, section)"),
        ("Tab", "Next field / tree ↔ table"),
        ("t", "Toggle section tree"),
        ("Enter", "Detail view"),
        ("a", "Add fact"),
        ("e", "Edit selected"),
        ("d", "Delete selected"),
        ("m", "Move selected"),
        ("x", "Check selected fact"),
        ("c / C", "Check all / cancel"),
        ("r", "Refresh from disk"),
        ("?", "This help"),
        ("q / Esc", "Quit"),
    ];

    let rows: Vec<Row> = bindings
        .iter()
        .map(|(key, action)| {
            Row::new(vec![
                Cell::from(*key).style(
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(*action).style(Style::default().fg(theme::TEXT)),
            ])
        })
        .collect();

    let table = Table::new(rows, [Constraint::Length(14), Constraint::Fill(1)])
        .style(Style::default().bg(theme::BG));
    frame.render_widget(table, chunks[0]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            " FACTS ",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "— fact-driven development",
            Style::default().fg(theme::MUTED),
        ),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme::BG));
    frame.render_widget(footer, chunks[1]);
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let fact = match app
        .detail_fact
        .and_then(|gi| app.display_facts.iter().find(|f| f.global_index == gi))
    {
        Some(f) => f,
        None => return,
    };

    let has_stderr = matches!(
        app.check_results.get(&fact.global_index),
        Some(CheckResult::Failed(s)) if !s.is_empty()
    );
    let detail_area = if has_stderr {
        centered_rect(70, 65, area)
    } else {
        centered_rect(50, 40, area)
    };
    frame.render_widget(Clear, detail_area);

    let block = Block::default()
        .title(Line::from(Span::styled(
            " FACT DETAIL ",
            Style::default()
                .fg(theme::PURPLE)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG));

    let inner = block.inner(detail_area);
    frame.render_widget(block, detail_area);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("ID:      ", Style::default().fg(theme::MUTED)),
            Span::styled(fact.id.clone(), Style::default().fg(theme::ACCENT)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Label:   ", Style::default().fg(theme::MUTED)),
            Span::styled(fact.label.clone(), Style::default().fg(theme::TEXT)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Section: ", Style::default().fg(theme::MUTED)),
            Span::styled(fact.display_path(), Style::default().fg(theme::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("File:    ", Style::default().fg(theme::MUTED)),
            Span::styled(fact.filename.clone(), Style::default().fg(theme::TEXT)),
        ]),
    ];

    if let Some(ref cmd) = fact.command {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Command: ", Style::default().fg(theme::MUTED)),
            Span::styled(cmd.clone(), Style::default().fg(theme::TEXT)),
        ]));
    }

    if !fact.tags.is_empty() {
        let tags_str = fact
            .tags
            .iter()
            .map(|t| format!("@{t}"))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Tags:    ", Style::default().fg(theme::MUTED)),
            Span::styled(tags_str, Style::default().fg(theme::PASS)),
        ]));
    }

    if let Some(result) = app.check_results.get(&fact.global_index) {
        let (icon, style) = match result {
            CheckResult::Passed => ("✓ PASSED", Style::default().fg(theme::PASS)),
            CheckResult::Failed(_) => ("✗ FAILED", Style::default().fg(theme::FAIL)),
            CheckResult::Manual => ("? MANUAL", Style::default().fg(theme::MANUAL)),
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Status:  ", Style::default().fg(theme::MUTED)),
            Span::styled(icon, style),
        ]));
    }

    if let Some(CheckResult::Failed(stderr)) = app.check_results.get(&fact.global_index)
        && !stderr.is_empty()
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "─── stderr ───",
            Style::default()
                .fg(theme::FAIL)
                .add_modifier(Modifier::BOLD),
        )));
        let line_count = stderr.lines().count();
        for line in stderr.lines().take(20) {
            lines.push(Line::from(Span::styled(
                format!(" {line}"),
                Style::default().fg(theme::MUTED),
            )));
        }
        if line_count > 20 {
            lines.push(Line::from(Span::styled(
                format!("  … ({} more lines)", line_count - 20),
                Style::default().fg(theme::MUTED),
            )));
        }
    }

    let detail = Paragraph::new(lines)
        .style(Style::default().bg(theme::BG))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, inner);
}

pub fn draw_form(frame: &mut Frame, form: &FormState, area: Rect) {
    let form_height = (form.fields.len() as u16) * 2 + 5;
    let form_width = 56u16.min(area.width.saturating_sub(4));
    let form_area = centered_rect_fixed(form_width, form_height, area);

    frame.render_widget(Clear, form_area);

    let block = Block::default()
        .title(Line::from(Span::styled(
            format!(" {} ", form.title),
            Style::default()
                .fg(theme::PURPLE)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG));

    let inner = block.inner(form_area);
    frame.render_widget(block, form_area);

    let mut constraints: Vec<Constraint> = Vec::new();
    constraints.push(Constraint::Length(1)); // top padding
    for _ in &form.fields {
        constraints.push(Constraint::Length(1)); // field
        constraints.push(Constraint::Length(1)); // gap
    }
    constraints.push(Constraint::Length(1)); // error
    constraints.push(Constraint::Length(1)); // footer
    constraints.push(Constraint::Fill(1));

    let chunks = Layout::vertical(constraints).split(inner);

    for (i, field) in form.fields.iter().enumerate() {
        let is_focused = i == form.focused;
        let row = chunks[1 + i * 2];

        let field_chunks =
            Layout::horizontal([Constraint::Length(10), Constraint::Fill(1)]).split(row);

        let label_style = if is_focused {
            Style::default().fg(theme::ACCENT)
        } else if field.required && field.value.is_empty() {
            Style::default().fg(theme::FAIL)
        } else {
            Style::default().fg(theme::MUTED)
        };

        let label_text = if field.required {
            format!(" {}*", field.label)
        } else {
            format!(" {}", field.label)
        };
        frame.render_widget(
            Paragraph::new(label_text).style(label_style),
            field_chunks[0],
        );

        let input_style = if is_focused {
            Style::default().fg(theme::TEXT).bg(theme::SURFACE)
        } else {
            Style::default().fg(theme::TEXT)
        };
        frame.render_widget(
            Paragraph::new(format!(" {}", field.value)).style(input_style),
            field_chunks[1],
        );

        if is_focused {
            let cursor_w = display_width_before(&field.value, field.cursor);
            let cx = field_chunks[1].x + 1 + cursor_w as u16;
            frame.set_cursor_position(Position::new(
                cx.min(field_chunks[1].right().saturating_sub(1)),
                field_chunks[1].y,
            ));
        }
    }

    let error_row = 1 + form.fields.len() * 2;
    if let Some(ref err) = form.error {
        frame.render_widget(
            Paragraph::new(format!(" {err}")).style(Style::default().fg(theme::FAIL)),
            chunks[error_row],
        );
    }

    let footer_row = error_row + 1;
    if footer_row < chunks.len() {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(" Ctrl-S", Style::default().fg(theme::ACCENT)),
            Span::styled(" submit  ", Style::default().fg(theme::MUTED)),
            Span::styled("Tab", Style::default().fg(theme::ACCENT)),
            Span::styled(" next  ", Style::default().fg(theme::MUTED)),
            Span::styled("Esc", Style::default().fg(theme::ACCENT)),
            Span::styled(" cancel", Style::default().fg(theme::MUTED)),
        ]));
        frame.render_widget(footer, chunks[footer_row]);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(v[1])[1]
}

fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.x + r.width.saturating_sub(width) / 2;
    let y = r.y + r.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(r.width), height.min(r.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn temp_app(content: &str) -> (TempDir, App) {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".facts"), content).unwrap();
        let app = App::new(dir.path().to_path_buf(), false).unwrap();
        (dir, app)
    }

    #[test]
    fn test_full_labels_on_wide_terminal() {
        let items = vec![
            ("↑↓", "nav"),
            ("/", "filter"),
            ("Enter", "detail"),
            ("a", "add"),
            ("e", "edit"),
            ("d", "del"),
            ("q", "quit"),
        ];
        let spans = build_shortcut_spans(&items, 100);
        let text = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("nav"));
        assert!(text.contains("filter"));
        assert!(text.contains("detail"));
        assert!(text.contains("add"));
        assert!(text.contains("edit"));
        assert!(text.contains("del"));
        assert!(text.contains("quit"));
    }

    #[test]
    fn test_keys_only_on_medium_terminal() {
        let items = vec![
            ("↑↓", "nav"),
            ("/", "filter"),
            ("Enter", "detail"),
            ("a", "add"),
        ];
        // Full labels exceed width 20; keys-only (width 14) fits.
        let spans = build_shortcut_spans(&items, 20);
        let text = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("↑↓"));
        assert!(!text.contains("nav"));
        assert!(!text.contains("filter"));
        assert!(!text.contains("detail"));
        assert!(!text.contains("add"));
    }

    #[test]
    fn test_truncation_on_narrow_terminal() {
        let items = vec![
            ("↑↓", "nav"),
            ("/", "filter"),
            ("Enter", "detail"),
            ("a", "add"),
            ("q", "quit"),
        ];
        // Keys-only is 16 chars; at width 10 it must truncate.
        let spans = build_shortcut_spans(&items, 10);
        let text = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("↑↓"));
        assert!(!text.contains("Enter"));
        assert!(!text.contains("quit"));
    }

    #[test]
    fn test_ellipsis_when_partially_truncated() {
        let items = vec![
            ("↑↓", "nav"),
            ("/", "filter"),
            ("Enter", "detail"),
            ("Ctrl-S", "submit"),
        ];
        // Keys-only is 19 chars; at width 16 we truncate off " Ctrl-S" (7), leaving 12,
        // and have room for " …" (2) since 12+2=14 <= 16.
        let spans = build_shortcut_spans(&items, 16);
        let text = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("…"));
        assert!(text.contains("↑↓"));
        assert!(!text.contains("Ctrl-S"));
    }

    #[test]
    fn test_no_ellipsis_when_too_narrow() {
        let items = vec![("↑↓", "nav"), ("/", "filter"), ("Enter", "detail")];
        // Keys-only is 12 chars; at width 4 we can only fit the leading space + first key (4).
        let spans = build_shortcut_spans(&items, 4);
        let text = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert!(text.contains("↑↓"));
        assert!(!text.contains("…"));
    }

    #[test]
    fn test_shortcut_items_browse_mode() {
        let (_dir, app) = temp_app("- first\n- second\n");
        let items = shortcut_items_for_mode(&app);
        assert!(items.iter().any(|(k, l)| *k == "↑↓" && *l == "nav"));
        assert!(items.iter().any(|(k, l)| *k == "a" && *l == "add"));
        assert!(items.iter().any(|(k, l)| *k == "q" && *l == "quit"));
        assert!(!items.iter().any(|(k, _)| *k == "Tab"));
    }

    #[test]
    fn test_shortcut_items_browse_with_tree() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.show_tree = true;
        let items = shortcut_items_for_mode(&app);
        assert!(items.iter().any(|(k, _)| *k == "Tab"));
    }

    #[test]
    fn test_shortcut_items_filter_mode() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        let items = shortcut_items_for_mode(&app);
        assert!(items.iter().any(|(k, l)| *k == "Tab" && *l == "next field"));
        assert!(items.iter().any(|(k, l)| *k == "Esc" && *l == "cancel"));
    }

    // === Unicode cursor positioning ===

    #[test]
    fn test_display_width_before_ascii() {
        assert_eq!(display_width_before("hello", 3), 3);
    }

    #[test]
    fn test_display_width_before_latin1() {
        // "café" — é is U+00E9 (2 bytes, 1 display column)
        assert_eq!(display_width_before("café", 5), 4);
    }

    #[test]
    fn test_display_width_before_cjk() {
        // "中文" — each CJK char is 3 bytes, 2 display columns
        assert_eq!(display_width_before("中文", 3), 2);
        assert_eq!(display_width_before("中文", 6), 4);
    }

    #[test]
    fn test_display_width_before_emoji() {
        // "a😀b" — 😀 is 4 bytes, 2 display columns
        assert_eq!(display_width_before("a😀b", 5), 3); // a + 😀
        assert_eq!(display_width_before("a😀b", 6), 4); // a + 😀 + b
    }

    #[test]
    fn test_display_width_before_zero_width() {
        // "a\u{0300}b" — combining grave accent is 0 display columns
        assert_eq!(display_width_before("a\u{0300}b", 4), 2);
    }

    #[test]
    fn test_display_width_before_clamps_out_of_bounds() {
        assert_eq!(display_width_before("hi", 10), 2);
    }

    #[test]
    fn test_filter_prefix_width_field0() {
        let (_dir, app) = temp_app("- first\n- second\n");
        assert_eq!(filter_prefix_width(&app), 2); // " /"
    }

    #[test]
    fn test_filter_prefix_width_field1_empty_search() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 1;
        assert_eq!(filter_prefix_width(&app), 5); // " /" + "" + "  @"
    }

    #[test]
    fn test_filter_prefix_width_field1_ascii_search() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.search = "foo".into();
        app.filter_field = 1;
        assert_eq!(filter_prefix_width(&app), 8); // " /" + "foo" + "  @"
    }

    #[test]
    fn test_filter_prefix_width_field1_unicode_search() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.search = "café".into();
        app.filter_field = 1;
        assert_eq!(filter_prefix_width(&app), 9); // " /" + "café" (4 cols) + "  @"
    }

    #[test]
    fn test_filter_prefix_width_field2_empty() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.filter_field = 2;
        assert_eq!(filter_prefix_width(&app), 5); // " /" + "" + "  #"
    }

    #[test]
    fn test_filter_prefix_width_field2_with_tags() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.search = "foo".into();
        app.tags_filter = "mv".into();
        app.filter_field = 2;
        assert_eq!(filter_prefix_width(&app), 13); // " /" + "foo" + "  @" + "mv" + "  #"
    }

    #[test]
    fn test_filter_prefix_width_field2_unicode_tags() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.search = "café".into();
        app.tags_filter = "中".into();
        app.filter_field = 2;
        assert_eq!(filter_prefix_width(&app), 14); // " /" + 4 + "  @" + 2 + "  #"
    }

    #[test]
    fn test_filter_prefix_width_field2_cjk_search() {
        let (_dir, mut app) = temp_app("- first\n- second\n");
        app.mode = Mode::Filter;
        app.search = "中文".into();
        app.filter_field = 2;
        assert_eq!(filter_prefix_width(&app), 9); // " /" + 4 + "  #"
    }
}
