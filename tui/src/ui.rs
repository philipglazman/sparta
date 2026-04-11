use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table},
    Frame,
};

use crate::app::{App, Direction, FetchStatus, PnlField};

const GREEN: Color = Color::Rgb(74, 222, 128);
const RED: Color = Color::Rgb(248, 113, 113);
const BTC_ORANGE: Color = Color::Rgb(247, 147, 26);
const DIM: Color = Color::DarkGray;

fn change_color(value: f64) -> Color {
    if value >= 0.0 { GREEN } else { RED }
}

fn format_usd(value: f64) -> String {
    // Format with commas and 2 decimal places
    let s = format!("{:.2}", value);
    let parts: Vec<&str> = s.split('.').collect();
    let integer = parts[0];
    let decimal = parts[1];

    let negative = integer.starts_with('-');
    let digits: String = integer.chars().filter(|c| c.is_ascii_digit()).collect();

    // Build comma-separated string by reversing, inserting commas, then reversing back
    let reversed_with_commas: String = digits
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| {
            if i > 0 && i % 3 == 0 {
                format!(",{}", c)
            } else {
                c.to_string()
            }
        })
        .collect::<Vec<_>>()
        .concat();
    let with_commas: String = reversed_with_commas.chars().rev().collect();

    if negative {
        format!("-${}.{}", with_commas, decimal)
    } else {
        format!("${}.{}", with_commas, decimal)
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(6),  // Price panel
        Constraint::Length(6),  // % Move calculator
        Constraint::Length(5),  // Open/Close
        Constraint::Length(13), // PNL calculator
        Constraint::Length(3),  // Links bar
        Constraint::Min(4),    // Debug log
    ])
    .split(area);

    // --- Debug Log (always rendered) ---
    let log_lines: Vec<Line> = app
        .logs
        .iter()
        .rev()
        .take((chunks[5].height.saturating_sub(2)) as usize)
        .rev()
        .map(|l| Line::from(Span::styled(format!(" {}", l), Style::default().fg(DIM))))
        .collect();

    let log_panel = Paragraph::new(log_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Debug Log ", Style::default().fg(Color::Yellow))),
    );
    frame.render_widget(log_panel, chunks[5]);

    match &app.data {
        None => {
            let status_msg = match &app.status {
                FetchStatus::Loading => "Loading...".to_string(),
                FetchStatus::Error(e) => format!("Error: {}", e),
                FetchStatus::Ok => "Loading...".to_string(),
            };
            let loading = Paragraph::new(status_msg)
                .style(Style::default().fg(DIM))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(Span::styled(" ₿ Bitcoin ", Style::default().fg(BTC_ORANGE))),
                );
            frame.render_widget(loading, chunks[0]);
        }
        Some(data) => {
            // --- Price Panel ---
            let status_text = match &app.status {
                FetchStatus::Ok => format!(
                    "Updated {} · Next refresh: {}s",
                    data.last_updated.format("%H:%M:%S UTC"),
                    app.seconds_until_refresh
                ),
                FetchStatus::Error(e) => format!("⚠ Fetch failed: {} · Retry: {}s", e, app.seconds_until_refresh),
                FetchStatus::Loading => "Refreshing...".to_string(),
            };

            let price_text = vec![
                Line::from(vec![
                    Span::styled(" BTC  ", Style::default().fg(BTC_ORANGE).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format_usd(data.price),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{:+.2}%", data.btc_daily_change_pct()),
                        Style::default().fg(change_color(data.btc_daily_change_pct())),
                    ),
                    Span::styled(" today", Style::default().fg(DIM)),
                ]),
                Line::from(vec![
                    Span::styled(" ETH  ", Style::default().fg(Color::Rgb(98, 126, 234)).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format_usd(data.eth_price),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{:+.2}%", data.eth_daily_change_pct()),
                        Style::default().fg(change_color(data.eth_daily_change_pct())),
                    ),
                    Span::styled(" today", Style::default().fg(DIM)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    format!(" {}", status_text),
                    Style::default().fg(DIM),
                )),
            ];

            let price_panel = Paragraph::new(price_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" Prices ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
            );
            frame.render_widget(price_panel, chunks[0]);

            // --- % Move Calculator ---
            let moves = data.percentage_moves();
            let rows: Vec<Row> = moves
                .iter()
                .map(|m| {
                    Row::new(vec![
                        Line::from(format!("{}%", m.percent)),
                        Line::from(Span::styled(format_usd(m.price_up), Style::default().fg(GREEN))),
                        Line::from(Span::styled(format_usd(m.price_down), Style::default().fg(RED))),
                    ])
                })
                .collect();

            let table = Table::new(
                rows,
                [Constraint::Length(8), Constraint::Length(20), Constraint::Length(20)],
            )
            .header(
                Row::new(vec!["Move", "Up", "Down"])
                    .style(Style::default().fg(DIM).add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" % Move Calculator ", Style::default().fg(Color::White))),
            );
            frame.render_widget(table, chunks[1]);

            // --- Open/Close ---
            let weekly_pct = data.weekly_change_pct();
            let monthly_pct = data.monthly_change_pct();
            let oc_text = vec![
                Line::from(vec![
                    Span::styled(" Week Open  ", Style::default().fg(DIM)),
                    Span::styled(
                        format!("({})", data.weekly_open_date.format("%b %d")),
                        Style::default().fg(DIM),
                    ),
                    Span::raw(format!("  {}  ", format_usd(data.weekly_open))),
                    Span::styled(
                        format!("{:+.2}%", weekly_pct),
                        Style::default().fg(change_color(weekly_pct)),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(" Month Open ", Style::default().fg(DIM)),
                    Span::styled(
                        format!("({})", data.monthly_open_date.format("%b %d")),
                        Style::default().fg(DIM),
                    ),
                    Span::raw(format!("  {}  ", format_usd(data.monthly_open))),
                    Span::styled(
                        format!("{:+.2}%", monthly_pct),
                        Style::default().fg(change_color(monthly_pct)),
                    ),
                ]),
            ];

            let oc_panel = Paragraph::new(oc_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" Open/Close ", Style::default().fg(Color::White))),
            );
            frame.render_widget(oc_panel, chunks[2]);

            // --- PNL Calculator ---
            let pnl = &app.pnl;
            let dir_str = match pnl.direction {
                Direction::Long => "LONG",
                Direction::Short => "SHORT",
            };
            let dir_color = match pnl.direction {
                Direction::Long => GREEN,
                Direction::Short => RED,
            };

            let highlight = |field: PnlField, label: &str, val: &str| -> Line {
                let is_focused = pnl.active && pnl.focused_field == field;
                let label_style = if is_focused {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(DIM)
                };
                let val_style = if is_focused {
                    Style::default().fg(Color::White).add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(vec![
                    Span::styled(format!(" {}: ", label), label_style),
                    Span::styled(val.to_string(), val_style),
                ])
            };

            let mut pnl_lines = vec![
                Line::from(vec![
                    Span::styled(" Direction: ", Style::default().fg(DIM)),
                    Span::styled(dir_str, Style::default().fg(dir_color).add_modifier(Modifier::BOLD)),
                    Span::styled("  (d to toggle)", Style::default().fg(DIM)),
                ]),
                highlight(PnlField::Entry, "Entry", &if pnl.entry_buf.is_empty() { "—".to_string() } else { format!("${}", pnl.entry_buf) }),
                highlight(PnlField::Value, "Value", &if pnl.value_buf.is_empty() { "—".to_string() } else { format!("${}", pnl.value_buf) }),
                highlight(PnlField::Target, "Target", &if pnl.target_buf.is_empty() { "—".to_string() } else { format!("${}", pnl.target_buf) }),
            ];

            // Show results if we have data
            if let Some(price) = app.data.as_ref().map(|d| d.price) {
                let results = pnl.results(price);
                if !results.is_empty() {
                    pnl_lines.push(Line::from(""));
                    for r in &results {
                        let color = change_color(r.pnl);
                        pnl_lines.push(Line::from(vec![
                            Span::styled(format!(" {:>8} ", r.label), Style::default().fg(DIM)),
                            Span::raw(format!("{}  ", format_usd(r.price))),
                            Span::styled(
                                format!("{}{} ({:+.1}%)", if r.pnl >= 0.0 { "+" } else { "" }, format_usd(r.pnl), r.pnl_pct),
                                Style::default().fg(color),
                            ),
                        ]));
                    }
                }
            }

            let pnl_border_style = if pnl.active {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let pnl_title = if pnl.active {
                " PNL Calculator (Tab: next, Esc: deselect) "
            } else {
                " PNL Calculator (p to edit) "
            };

            let pnl_panel = Paragraph::new(pnl_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(pnl_border_style)
                    .title(Span::styled(pnl_title, Style::default().fg(Color::White))),
            );
            frame.render_widget(pnl_panel, chunks[3]);

            // --- Links Bar ---
            let links_text = Line::from(vec![
                Span::styled(" [1] ", Style::default().fg(Color::Yellow)),
                Span::raw("Velo  "),
                Span::styled("[2] ", Style::default().fg(Color::Yellow)),
                Span::raw("TradingEconomics  "),
                Span::styled("[3] ", Style::default().fg(Color::Yellow)),
                Span::raw("MangoWorks  "),
                Span::styled("[4] ", Style::default().fg(Color::Yellow)),
                Span::raw("Kiyotaka"),
            ]);

            let links_panel = Paragraph::new(links_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(" Links ", Style::default().fg(Color::White))),
            );
            frame.render_widget(links_panel, chunks[4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_usd() {
        assert_eq!(format_usd(87420.50), "$87,420.50");
        assert_eq!(format_usd(100000.0), "$100,000.00");
        assert_eq!(format_usd(1234.56), "$1,234.56");
        assert_eq!(format_usd(999.99), "$999.99");
        assert_eq!(format_usd(0.0), "$0.00");
    }

    #[test]
    fn test_change_color() {
        assert_eq!(change_color(1.0), GREEN);
        assert_eq!(change_color(-1.0), RED);
        assert_eq!(change_color(0.0), GREEN);
    }
}
