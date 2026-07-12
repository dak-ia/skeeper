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

/// セッション一覧をTUIで表示、選択されたセッションのindexを返す。
/// Escまたはqでキャンセルされた場合は `Ok(None)`
pub fn pick_session(sessions: &[SessionMeta]) -> Result<Option<usize>> {
    if sessions.is_empty() {
        return Ok(None);
    }

    let _guard = TerminalGuard::enter()?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(stdout())).context("Failed to initialize terminal")?;

    let offset = display::local_offset();

    let mut state = ListState::default();
    state.select(Some(0));

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

            let header = Paragraph::new(" skeeper — Select a session ")
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, layout[0]);

            let items: Vec<ListItem> = sessions
                .iter()
                .map(|s| ListItem::new(session_line_text(s, offset)))
                .collect();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" Sessions "))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("▶ ");
            frame.render_stateful_widget(list, layout[1], &mut state);

            let footer = Paragraph::new("↑↓ Navigate   Enter Attach   Esc/q Cancel");
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
                let i = state.selected().unwrap_or(0);
                state.select(Some(if i == 0 { sessions.len() - 1 } else { i - 1 }));
            }
            KeyCode::Down => {
                let i = state.selected().unwrap_or(0);
                state.select(Some((i + 1) % sessions.len()));
            }
            KeyCode::Enter => {
                return Ok(state.selected());
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                return Ok(None);
            }
            // raw mode下ではCtrl+CはKeyEventとして届く(SIGINT化されない)ので明示的に受ける。
            // 「操作を奪わない」方針として、とっさに押されるCtrl+C / Ctrl+Dも終了手段として受け付ける
            KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            _ => {}
        }
    }
}
