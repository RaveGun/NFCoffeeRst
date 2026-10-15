//! Terminal rendering (ratatui) for every screen and overlay.

use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{Action, App, Form, Hotspot, Overlay, Tab};

const SPIN: [&str; 4] = ["|", "/", "-", "\\"];

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hotspots.clear();
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    let modal = app.edit.is_some();
    draw_tabs(frame, app, chunks[0], !modal);

    if let Some(form) = app.edit.clone() {
        frame.render_widget(Clear, chunks[1]);
        draw_form(frame, app, chunks[1], &form, false);
    } else {
        match app.tab {
            Tab::Dashboard => draw_dashboard(frame, app, chunks[1]),
            Tab::Users => draw_users(frame, app, chunks[1]),
            Tab::Settings => {
                let form = app.settings_form.clone();
                draw_form(frame, app, chunks[1], &form, true);
            }
        }
    }

    draw_footer(frame, app, chunks[2]);

    if let Some(overlay) = app.overlay.clone() {
        app.hotspots.clear();
        draw_overlay(frame, app, area, &overlay);
    }
}

fn draw_tabs(frame: &mut Frame, app: &mut App, area: Rect, interactive: bool) {
    let tabs = [
        ("F2 Dashboard", Tab::Dashboard),
        ("F3 Users", Tab::Users),
        ("F4 Settings", Tab::Settings),
    ];
    let mut spans: Vec<Span> = Vec::new();
    let mut x = area.x;
    for (label, tab) in tabs {
        let text = format!(" {label} ");
        let w = text.chars().count() as u16;
        let style = if app.tab == tab {
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
        if interactive {
            app.hotspots.push(Hotspot {
                area: Rect::new(x, area.y, w, 1),
                action: Action::Tab(tab),
            });
        }
        x += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) {
    let hint = match app.tab {
        Tab::Dashboard => "[T]ransfer  [C]lear  [E]xport  [I]mport  F1 help  Ctrl+Q quit",
        Tab::Users => "Up/Down select  Enter edit  [N]ew  [D]elete  [C]lear  [/]search",
        Tab::Settings => "Tab/Enter next  Ctrl+S save  Esc revert",
    };
    let state = if app.transferring {
        format!("{} transferring", SPIN[app.spinner % SPIN.len()])
    } else {
        app.status.clone()
    };
    let line = Line::from(vec![
        Span::styled(format!(" {state} "), Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(hint, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// --- dashboard --------------------------------------------------------------

fn draw_dashboard(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(3),
        Constraint::Min(5),
    ])
    .split(area);

    let cfg = app.cfg.clone();
    let transfer_state = if app.transferring {
        format!("{} transfer in progress", SPIN[app.spinner % SPIN.len()])
    } else {
        "idle".to_string()
    };
    let pre = if cfg.pre_connect_cmd.is_empty() {
        "<auto / OS>".to_string()
    } else {
        cfg.pre_connect_cmd.clone()
    };
    let status_lines = vec![
        Line::from(format!("Device      : {}:{}", cfg.host, cfg.port)),
        Line::from(format!(
            "WiFi AP     : {}   (password hidden)",
            cfg.ssid
        )),
        Line::from(format!("Pre-connect : {pre}")),
        Line::from(format!("State       : {transfer_state}")),
        Line::from(format!("Users in DB : {}", app.users.len())),
    ];
    frame.render_widget(
        Paragraph::new(status_lines).block(Block::bordered().title(" Device / Status ")),
        chunks[0],
    );

    draw_buttons(frame, app, chunks[1]);
    draw_log(frame, app, chunks[2]);
}

fn draw_buttons(frame: &mut Frame, app: &mut App, area: Rect) {
    let defs = [
        ("Transfer (T)", Action::StartTransfer, Color::Green),
        ("Clear counters (C)", Action::ClearCounters, Color::Yellow),
        ("Export CSV (E)", Action::Export, Color::Cyan),
        ("Import CSV (I)", Action::Import, Color::Cyan),
        ("Help (F1)", Action::Help, Color::Magenta),
    ];
    draw_button_row(frame, app, area, &defs, " Actions ");
}

fn draw_users_actions(frame: &mut Frame, app: &mut App, area: Rect) {
    let defs = [
        ("New (N)", Action::NewUser, Color::Green),
        ("Edit (Enter)", Action::EditSelected, Color::Cyan),
        ("Delete (D)", Action::DeleteUser, Color::Red),
        ("Clear counters (C)", Action::ClearCounters, Color::Yellow),
        ("Search (/)", Action::Search, Color::Magenta),
    ];
    draw_button_row(frame, app, area, &defs, " User actions ");
}

fn draw_button_row(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    defs: &[(&str, Action, Color)],
    title: &str,
) {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let mut x = inner.x;
    let mut spans: Vec<Span> = Vec::new();
    for (label, action, color) in defs {
        let text = format!(" {label} ");
        let w = text.chars().count() as u16;
        spans.push(Span::styled(
            text,
            Style::default().fg(Color::Black).bg(*color),
        ));
        spans.push(Span::raw(" "));
        app.hotspots.push(Hotspot {
            area: Rect::new(x, inner.y, w, 1),
            action: action.clone(),
        });
        x += w + 1;
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .block(Block::bordered().title(title.to_string())),
        area,
    );
}

fn draw_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner_h = area.height.saturating_sub(2) as usize;
    let total = app.log.len();
    let max = total.saturating_sub(inner_h) as u16;
    if app.log_autoscroll || app.log_scroll > max {
        app.log_scroll = max;
    }
    let lines: Vec<Line> = app.log.iter().map(|l| Line::from(l.as_str())).collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(format!(" Log ({total}) ")))
            .scroll((app.log_scroll, 0)),
        area,
    );
}

// --- users ------------------------------------------------------------------

fn draw_users(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(area);

    let filter = if app.search.is_empty() {
        " No filter  (/ to search)".to_string()
    } else {
        format!(" Filter: {}  (Esc clears)", app.search)
    };
    frame.render_widget(
        Paragraph::new(filter)
            .block(Block::bordered().title(format!(" Users ({}) ", app.visible.len()))),
        chunks[0],
    );

    draw_users_actions(frame, app, chunks[1]);

    let list_area = chunks[2];
    frame.render_widget(Block::bordered(), list_area);
    let inner = list_area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let header = format!(
        "{:<16} {:<12} {:<22} {:<24} {:>6}",
        "NFCID", "CODENAME", "NAME", "EMAIL", "COFF"
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            header,
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let rows_area = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );
    let h = rows_area.height as usize;
    app.list_height = h.max(1) as u16;
    if app.selected < app.list_offset {
        app.list_offset = app.selected;
    }
    if h > 0 && app.selected >= app.list_offset + h {
        app.list_offset = app.selected + 1 - h;
    }

    for i in 0..h {
        let idx = app.list_offset + i;
        if idx >= app.visible.len() {
            break;
        }
        let u = &app.users[app.visible[idx]];
        let line = format!(
            "{:<16} {:<12} {:<22} {:<24} {:>6}",
            trunc(&u.nfcid, 16),
            trunc(&u.codename, 12),
            trunc(&u.uname, 22),
            trunc(&u.email, 24),
            u.coff
        );
        let style = if idx == app.selected {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default()
        };
        let row = Rect::new(rows_area.x, rows_area.y + i as u16, rows_area.width, 1);
        frame.render_widget(Paragraph::new(Line::from(Span::styled(line, style))), row);
        app.hotspots.push(Hotspot {
            area: row,
            action: Action::SelectRow(idx),
        });
    }
}

// --- forms ------------------------------------------------------------------

fn draw_form(frame: &mut Frame, app: &mut App, area: Rect, form: &Form, is_settings: bool) {
    let block = Block::bordered().title(format!(" {} ", form.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label_w = form
        .labels
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    for (i, label) in form.labels.iter().enumerate() {
        let field = match form.fields.get(i) {
            Some(f) => f,
            None => continue,
        };
        let y = inner.y + i as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let focused = i == form.focus;
        let prefix = format!(" {:<width$} : ", label, width = label_w);
        let value = field.display();
        let value_style = if focused {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };
        let line = Line::from(vec![
            Span::raw(prefix),
            Span::styled(format!("{value} "), value_style),
        ]);
        let row = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(Paragraph::new(line), row);
        app.hotspots.push(Hotspot {
            area: row,
            action: Action::FocusField(i),
        });
    }

    if let Some(field) = form.fields.get(form.focus) {
        let x = inner.x + label_w as u16 + 4 + field.cursor as u16;
        let y = inner.y + form.focus as u16;
        if x < inner.x + inner.width && y < inner.y + inner.height {
            frame.set_cursor_position(Position::new(x, y));
        }
    }

    // Buttons on the last inner row.
    let by = inner.y + inner.height.saturating_sub(1);
    let (save_action, cancel_action) = if is_settings {
        (Action::SaveSettings, Action::CancelSettings)
    } else {
        (Action::SaveEdit, Action::CancelEdit)
    };
    let save = " Save (Ctrl+S/Enter) ";
    let cancel = " Cancel (Esc) ";
    let save_w = save.chars().count() as u16;
    let cancel_w = cancel.chars().count() as u16;
    let line = Line::from(vec![
        Span::styled(
            save,
            Style::default().bg(Color::Green).fg(Color::Black),
        ),
        Span::raw("  "),
        Span::styled(cancel, Style::default().bg(Color::Red).fg(Color::Black)),
    ]);
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(inner.x, by, inner.width, 1),
    );
    app.hotspots.push(Hotspot {
        area: Rect::new(inner.x, by, save_w, 1),
        action: save_action,
    });
    app.hotspots.push(Hotspot {
        area: Rect::new(inner.x + save_w + 2, by, cancel_w, 1),
        action: cancel_action,
    });
}

// --- overlays ---------------------------------------------------------------

fn draw_overlay(frame: &mut Frame, app: &mut App, area: Rect, overlay: &Overlay) {
    match overlay {
        Overlay::Help { scroll } => {
            let w = (area.width * 3 / 4).clamp(40, area.width.max(40));
            let h = (area.height * 3 / 4).clamp(10, area.height.max(10));
            let rect = centered(area, w, h);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(help_text())
                    .block(Block::bordered().title(" Help  (F1 / Esc to close) "))
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0)),
                rect,
            );
        }
        Overlay::Confirm {
            title,
            message,
            selection,
            ..
        } => {
            let w = 60u16.min(area.width.saturating_sub(2)).max(24);
            let h = (message.len() as u16 + 4).min(area.height);
            let rect = centered(area, w, h);
            frame.render_widget(Clear, rect);
            frame.render_widget(Block::bordered().title(format!(" {title} ")), rect);
            let inner = rect.inner(Margin {
                vertical: 1,
                horizontal: 2,
            });
            let mut lines: Vec<Line> = message.iter().map(|m| Line::from(m.as_str())).collect();
            lines.push(Line::from(""));
            let yes_style = if *selection == 0 {
                Style::default().bg(Color::Green).fg(Color::Black)
            } else {
                Style::default().fg(Color::Gray)
            };
            let no_style = if *selection == 1 {
                Style::default().bg(Color::Red).fg(Color::Black)
            } else {
                Style::default().fg(Color::Gray)
            };
            lines.push(Line::from(vec![
                Span::styled(" Yes ", yes_style),
                Span::raw("   "),
                Span::styled(" No ", no_style),
            ]));
            let by = inner.y + message.len() as u16 + 1;
            frame.render_widget(Paragraph::new(lines), inner);
            app.hotspots.push(Hotspot {
                area: Rect::new(inner.x, by, 5, 1),
                action: Action::ConfirmYes,
            });
            app.hotspots.push(Hotspot {
                area: Rect::new(inner.x + 8, by, 4, 1),
                action: Action::ConfirmNo,
            });
        }
        Overlay::Input {
            title,
            label,
            input,
            ..
        } => {
            let w = 72u16.min(area.width.saturating_sub(2)).max(30);
            let h = 7u16.min(area.height);
            let rect = centered(area, w, h);
            frame.render_widget(Clear, rect);
            frame.render_widget(Block::bordered().title(format!(" {title} ")), rect);
            let inner = rect.inner(Margin {
                vertical: 1,
                horizontal: 2,
            });
            frame.render_widget(
                Paragraph::new(Line::from(label.as_str())),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let box_y = inner.y + 2;
            let box_rect = Rect::new(inner.x, box_y, inner.width, 1);
            frame.render_widget(
                Paragraph::new(format!("{} ", input.display()))
                    .style(Style::default().fg(Color::Black).bg(Color::White)),
                box_rect,
            );
            let cx = (inner.x + input.cursor as u16).min(inner.x + inner.width.saturating_sub(1));
            frame.set_cursor_position(Position::new(cx, box_y));

            let by = box_y + 2;
            if by < inner.y + inner.height {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" OK (Enter) ", Style::default().bg(Color::Green).fg(Color::Black)),
                        Span::raw("  "),
                        Span::styled(" Cancel (Esc) ", Style::default().bg(Color::Red).fg(Color::Black)),
                    ])),
                    Rect::new(inner.x, by, inner.width, 1),
                );
            }
            app.hotspots.push(Hotspot {
                area: Rect::new(inner.x, by, 12, 1),
                action: Action::InputSubmit,
            });
            app.hotspots.push(Hotspot {
                area: Rect::new(inner.x + 14, by, 15, 1),
                action: Action::InputCancel,
            });
        }
    }
}

// --- helpers ----------------------------------------------------------------

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        s.chars().take(n).collect()
    } else {
        s.to_string()
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn help_text() -> Vec<Line<'static>> {
    let rows = [
        "NFCoffee Manager - help",
        "",
        "Tabs:      F2 Dashboard   F3 Users   F4 Settings   F1 help",
        "Quit:      Ctrl+Q or Ctrl+C",
        "",
        "Dashboard:",
        "  T / Enter   start the data transfer from the device",
        "  C           clear all coffee counters",
        "  E / I       export / import CSV (Android format)",
        "  Up/Down, PgUp/PgDn, Home/End   scroll the log",
        "",
        "Users:",
        "  Up/Down     move selection       Enter  edit selected",
        "  N           new user/card        D      delete selected",
        "  C           clear all counters   /      search filter",
        "",
        "Manual card entry: type an NFCID (hex) OR a CODENAME;",
        "the other value is derived automatically on save.",
        "",
        "Transfer flow:",
        "  1. present the master card to the reader",
        "  2. press T - the app joins the AP (nmcli/netsh or your",
        "     pre-connect command) and streams the counters",
        "  3. received counts are ADDED to the stored values",
        "",
        "Press F1 or Esc to close this window.",
    ];
    rows.iter().map(|r| Line::from(*r)).collect()
}
