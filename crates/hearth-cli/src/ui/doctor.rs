//! Doctor checks.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::theme;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Doctor "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.doctor.running {
        frame.render_widget(
            Paragraph::new(Line::styled(
                if app.doctor.live {
                    "Running live checks (provider and Telegram)..."
                } else {
                    "Running offline checks..."
                },
                theme::warn(),
            )),
            inner,
        );
        return;
    }
    match &app.doctor.output {
        Some(output) => {
            // A scroll past the end must not render a blank pane, and the same
            // text stays visible while a new check runs.
            let total = output.lines().count();
            let offset = app.doctor.scroll.min(total.saturating_sub(1));
            let lines: Vec<&str> = output.lines().skip(offset).collect();
            frame.render_widget(
                Paragraph::new(lines.join("\n")).wrap(Wrap { trim: false }),
                inner,
            );
        }
        None => {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled(
                        "Check settings, the database, the lock, and the daemon.",
                        theme::muted(),
                    ),
                    Line::from(""),
                    Line::raw("Enter  run offline checks"),
                    Line::raw("l      run live checks (provider + Telegram)"),
                ])
                .wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
}
