use crate::{domain::Task, provider::FakePiAdapter, runtime::Runtime, store::Store};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{prelude::*, widgets::*};

pub async fn run(path: &std::path::Path) -> Result<()> {
    let runtime = Runtime::new(Store::open(path)?, FakePiAdapter);
    enable_raw_mode()?;
    let mut out = std::io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;
    let result = app_loop(&mut terminal, runtime).await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result
}

async fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    runtime: Runtime<FakePiAdapter>,
) -> Result<()> {
    let mut tasks = runtime.store.tasks()?;
    let mut input = String::new();
    loop {
        terminal.draw(|f| draw(f, &tasks, &input))?;
        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('q') if input.is_empty() => break,
                KeyCode::Enter if !input.trim().is_empty() => {
                    let task = runtime.submit(std::mem::take(&mut input))?;
                    tasks.push(runtime.run(task).await?);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame<'_>, tasks: &[Task], input: &str) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(5),
    ])
    .split(f.area());
    f.render_widget(
        Paragraph::new("ASTER  durable agent control plane   [Enter] submit  [q] quit")
            .block(Block::bordered()),
        areas[0],
    );
    let rows = tasks.iter().rev().map(|t| {
        Row::new(vec![
            t.id.to_string()[..8].into(),
            format!("{:?}", t.state),
            t.route.role.clone(),
            t.route.model.clone(),
            t.prompt.clone(),
        ])
    });
    f.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Min(20),
            ],
        )
        .header(Row::new(["ID", "STATE", "ROLE", "MODEL", "TASK"]).style(Style::new().bold()))
        .block(Block::bordered().title(" Durable tasks / routing trace ")),
        areas[1],
    );
    f.render_widget(
        Paragraph::new(input).block(Block::bordered().title(" Submit a task ")),
        areas[2],
    );
}
