use std::collections::HashSet;
use std::io::stdout;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::display;
use crate::session::SessionMeta;
use crate::term_guard::TerminalGuard;

use super::session_line_text;

/// セッション一覧をTUIで表示、複数選択されたセッションのindexをVecで返す(昇順)。
/// Enter時点で1件も選択されていなければ何もしない(操作継続)。
/// Esc/q/Ctrl+C/Ctrl+Dでキャンセルされた場合は `Ok(None)`
pub fn pick_sessions_multi(sessions: &[SessionMeta]) -> Result<Option<Vec<usize>>> {
    if sessions.is_empty() {
        return Ok(None);
    }

    let _guard = TerminalGuard::enter()?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(stdout())).context("Failed to initialize terminal")?;

    let offset = display::local_offset();

    let mut cursor = ListState::default();
    cursor.select(Some(0));
    let mut selected: HashSet<usize> = HashSet::new();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(area);

            let header_text = format!(
                " skeeper — Select sessions to kill  ({} selected) ",
                selected.len()
            );
            let header = Paragraph::new(header_text).block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, layout[0]);

            let items: Vec<ListItem> = sessions
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let marker = if selected.contains(&i) { "[x]" } else { "[ ]" };
                    let line = format!("{marker}  {}", session_line_text(s, offset));
                    ListItem::new(line)
                })
                .collect();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" Sessions "))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("▶ ");
            frame.render_stateful_widget(list, layout[1], &mut cursor);

            let footer = Paragraph::new(
                "↑↓ Move   Space Mark   Enter Kill (marked or current)   Esc/q Cancel",
            );
            frame.render_widget(footer, layout[2]);
        })?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up => {
                let i = cursor.selected().unwrap_or(0);
                cursor.select(Some(if i == 0 { sessions.len() - 1 } else { i - 1 }));
            }
            KeyCode::Down => {
                let i = cursor.selected().unwrap_or(0);
                cursor.select(Some((i + 1) % sessions.len()));
            }
            KeyCode::Char(' ') => {
                if let Some(i) = cursor.selected() {
                    if !selected.remove(&i) {
                        selected.insert(i);
                    }
                }
            }
            KeyCode::Enter => {
                if selected.is_empty() {
                    // Spaceで明示選択せずEnterを押した場合は、カーソル位置の1件だけを対象にする。
                    // fzf --multiと同じ挙動で、1件だけ選ぶ場合をSpaceなしで済ませられる
                    if let Some(i) = cursor.selected() {
                        return Ok(Some(vec![i]));
                    }
                    continue;
                }
                let mut indices: Vec<usize> = selected.iter().copied().collect();
                indices.sort_unstable();
                return Ok(Some(indices));
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                return Ok(None);
            }
            KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            _ => {}
        }
    }
}
