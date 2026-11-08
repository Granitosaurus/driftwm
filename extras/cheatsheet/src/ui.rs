use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::model::{App, Row};

const KEYS_MAX: usize = 40;
const WHAT_MAX: usize = 52;
/// Columns the raw-action column needs to be worth showing.
const RAW_MIN: usize = 14;

pub fn render(frame: &mut Frame, app: &mut App) {
    let block = Block::bordered().title(Line::from(vec![
        " driftwm keybindings ".bold(),
        format!("· mod = {} ", capitalized(&app.mod_key)).dim(),
    ]));
    let inner = block.inner(frame.area());
    frame.render_widget(block, frame.area());

    let [search, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::new().fg(Color::Cyan).bold()),
            if app.filter.is_empty() {
                "type to filter".dim()
            } else {
                Span::raw(app.filter.clone())
            },
            Span::styled("▏", Style::new().fg(Color::Cyan)),
        ])),
        search,
    );

    let rows = app.rows();
    let height = usize::from(body.height);
    let width = usize::from(body.width);
    let keys_w = column_width(&rows, KEYS_MAX, |e| &e.keys);
    let what_w = column_width(&rows, WHAT_MAX, |e| &e.what);
    let show_raw = width > 2 + keys_w + 2 + what_w + 2 + RAW_MIN;

    let max_scroll = rows.len().saturating_sub(height);
    let scroll = app.scroll.min(max_scroll);
    let lines: Vec<Line> = rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(|row| match row {
            Row::Header(group) => Line::from(Span::styled(
                group.title().to_uppercase(),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Row::Entry(e) => {
                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(
                        fit(&e.keys, keys_w),
                        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::raw(fit(&e.what, what_w)),
                ];
                if show_raw {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        fit(&e.raw, width - (2 + keys_w + 2 + what_w + 2)),
                        Style::new().add_modifier(Modifier::DIM),
                    ));
                }
                Line::from(spans)
            }
        })
        .collect();

    if lines.is_empty() {
        frame.render_widget(Paragraph::new("  no binding matches".dim()), body);
    } else {
        frame.render_widget(Paragraph::new(lines), body);
    }

    let entries = rows.iter().filter(|r| matches!(r, Row::Entry(_))).count();
    let total = app.entries.len();
    let count = if entries == total {
        format!("{total} bindings")
    } else {
        format!("{entries} of {total}")
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{count} · ↑↓ PgUp PgDn scroll · Esc clear, then quit"
        ))
        .dim(),
        footer,
    );

    app.viewport = height;
    app.scroll = scroll;
}

fn column_width(rows: &[Row], cap: usize, field: impl Fn(&crate::model::Entry) -> &str) -> usize {
    rows.iter()
        .filter_map(|r| match r {
            Row::Entry(e) => Some(field(e).chars().count()),
            Row::Header(_) => None,
        })
        .max()
        .unwrap_or(0)
        .min(cap)
}

/// Pad or cut `text` to exactly `width` columns, marking a cut with `…`.
fn fit(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len <= width {
        format!("{text:width$}")
    } else {
        let kept: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::model::{parse_reply, tests::SAMPLE};

    fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    fn app() -> App {
        App::new(&parse_reply(SAMPLE).unwrap())
    }

    #[test]
    fn shows_sections_keys_wording_and_raw_actions() {
        let lines = screen(&mut app(), 100, 30).join("\n");
        for needle in [
            "driftwm keybindings",
            "mod = Super",
            "APPS",
            "Super + Return",
            "Open a terminal",
            "exec-terminal",
            "MODIFIER TAPS",
            "Alt + Shift (tap)",
            "9 bindings",
        ] {
            assert!(lines.contains(needle), "missing {needle:?} in:\n{lines}");
        }
    }

    #[test]
    fn filter_narrows_the_list_and_the_count() {
        let mut app = app();
        app.edit_filter(|f| f.push_str("zoom"));
        let lines = screen(&mut app, 100, 30).join("\n");
        assert!(lines.contains("2 of 9"), "{lines}");
        assert!(lines.contains("Zoom in"));
        assert!(!lines.contains("Open a terminal"));
    }

    #[test]
    fn empty_result_says_so() {
        let mut app = app();
        app.edit_filter(|f| f.push_str("xyzzy"));
        assert!(
            screen(&mut app, 100, 20)
                .join("\n")
                .contains("no binding matches")
        );
    }

    #[test]
    fn scroll_is_clamped_to_the_last_page() {
        let mut app = app();
        app.scroll_by(10_000);
        let lines = screen(&mut app, 100, 12);
        assert!(app.scroll > 0 && app.scroll < 10_000);
        assert!(lines.join("\n").contains("brand-new-action"), "{lines:?}");
    }

    #[test]
    fn narrow_terminals_drop_the_raw_column_instead_of_overflowing() {
        let lines = screen(&mut app(), 52, 20).join("\n");
        assert!(lines.contains("Open a terminal"));
        assert!(!lines.contains("exec-terminal"));
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("ab", 4), "ab  ");
    }
}
