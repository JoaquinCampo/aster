use crate::{
    domain::{Task, TaskState},
    provider::FakePiAdapter,
    runtime::Runtime,
    store::Store,
};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{prelude::*, widgets::*};
use std::{
    io,
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Conversation,
    Tasks,
    Dag,
    Transcripts,
    Routing,
    Usage,
    Audit,
    Approvals,
    Context,
    Artifacts,
    Config,
    Memory,
    Providers,
    Plugins,
}

impl Screen {
    pub const ALL: [Self; 14] = [
        Self::Conversation,
        Self::Tasks,
        Self::Dag,
        Self::Transcripts,
        Self::Routing,
        Self::Usage,
        Self::Audit,
        Self::Approvals,
        Self::Context,
        Self::Artifacts,
        Self::Config,
        Self::Memory,
        Self::Providers,
        Self::Plugins,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Tasks => "Tasks",
            Self::Dag => "DAG",
            Self::Transcripts => "Transcripts",
            Self::Routing => "Routing/Overrides",
            Self::Usage => "Budgets/Usage",
            Self::Audit => "Audit",
            Self::Approvals => "Permissions/Approvals",
            Self::Context => "Context",
            Self::Artifacts => "Artifacts/Diffs/Evidence",
            Self::Config => "Config",
            Self::Memory => "Memory",
            Self::Providers => "Providers",
            Self::Plugins => "Plugins",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Model {
    pub screen: Screen,
    pub tasks: Vec<Task>,
    pub selected: usize,
    pub input: String,
    pub status: String,
    pub running: bool,
    pub tick: u64,
}

impl Model {
    pub fn new(tasks: Vec<Task>) -> Self {
        Self {
            screen: Screen::Conversation,
            tasks,
            selected: 0,
            input: String::new(),
            status: "ready".into(),
            running: true,
            tick: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    Tick,
    TasksLoaded(Vec<Task>),
    Submitted(Box<Task>),
    Error(String),
    Quit,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    None,
    Submit(String),
    Pause(uuid::Uuid),
    Resume(uuid::Uuid),
    Cancel(uuid::Uuid),
    Retry(uuid::Uuid),
    Override(uuid::Uuid),
    Quit,
}

pub fn update(model: &mut Model, msg: Msg) -> Cmd {
    match msg {
        Msg::Quit => {
            model.running = false;
            Cmd::Quit
        }
        Msg::Tick => {
            model.tick += 1;
            Cmd::None
        }
        Msg::TasksLoaded(tasks) => {
            model.tasks = tasks;
            model.selected = model.selected.min(model.tasks.len().saturating_sub(1));
            Cmd::None
        }
        Msg::Submitted(task) => {
            model.tasks.push(*task);
            model.selected = model.tasks.len() - 1;
            model.status = "task submitted".into();
            Cmd::None
        }
        Msg::Error(e) => {
            model.status = format!("error: {e}");
            Cmd::None
        }
        Msg::Key(key) if key.kind != KeyEventKind::Press => Cmd::None,
        Msg::Key(key) => update_key(model, key.code),
    }
}

fn update_key(model: &mut Model, code: KeyCode) -> Cmd {
    match code {
        KeyCode::Char('q') if model.input.is_empty() => {
            model.running = false;
            Cmd::Quit
        }
        KeyCode::Tab | KeyCode::Right => {
            let i = Screen::ALL
                .iter()
                .position(|s| *s == model.screen)
                .unwrap_or(0);
            model.screen = Screen::ALL[(i + 1) % Screen::ALL.len()];
            Cmd::None
        }
        KeyCode::BackTab | KeyCode::Left if model.input.is_empty() => {
            let i = Screen::ALL
                .iter()
                .position(|s| *s == model.screen)
                .unwrap_or(0);
            model.screen = Screen::ALL[(i + Screen::ALL.len() - 1) % Screen::ALL.len()];
            Cmd::None
        }
        KeyCode::Up => {
            model.selected = model.selected.saturating_sub(1);
            Cmd::None
        }
        KeyCode::Down => {
            model.selected = (model.selected + 1).min(model.tasks.len().saturating_sub(1));
            Cmd::None
        }
        KeyCode::Enter if !model.input.trim().is_empty() => {
            Cmd::Submit(std::mem::take(&mut model.input))
        }
        KeyCode::Backspace => {
            model.input.pop();
            Cmd::None
        }
        KeyCode::Char('p') if model.input.is_empty() => {
            action(model, "pause requested", Cmd::Pause)
        }
        KeyCode::Char('s') if model.input.is_empty() => {
            action(model, "resume requested", Cmd::Resume)
        }
        KeyCode::Char('x') if model.input.is_empty() => {
            action(model, "cancel requested", Cmd::Cancel)
        }
        KeyCode::Char('r') if model.input.is_empty() => {
            action(model, "retry requested", Cmd::Retry)
        }
        KeyCode::Char('o') if model.input.is_empty() => {
            action(model, "route override requested", Cmd::Override)
        }
        KeyCode::Char(c) => {
            model.input.push(c);
            Cmd::None
        }
        _ => Cmd::None,
    }
}
fn action(model: &mut Model, status: &str, f: impl FnOnce(uuid::Uuid) -> Cmd) -> Cmd {
    let Some(id) = model.tasks.get(model.selected).map(|t| t.id) else {
        model.status = "no task selected".into();
        return Cmd::None;
    };
    model.status = status.into();
    f(id)
}

struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub async fn run(path: &Path) -> Result<()> {
    let mut runtime = Runtime::new(Store::open(path)?, FakePiAdapter);
    let mut model = Model::new(runtime.store.tasks()?);
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let (tx, rx) = mpsc::channel();
    let event_tx = tx.clone();
    thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {
                    if let Ok(Event::Key(k)) = event::read()
                        && event_tx.send(Msg::Key(k)).is_err()
                    {
                        break;
                    }
                }
                Ok(false) => continue,
                Err(_) => break,
            }
        }
    });
    let mut next_tick = Instant::now();
    while model.running {
        terminal.draw(|f| view(f, &model))?;
        let msg = rx
            .recv_timeout(Duration::from_millis(50))
            .unwrap_or(Msg::Tick);
        let cmd = update(&mut model, msg);
        execute_cmd(&mut model, &mut runtime, cmd).await;
        if next_tick.elapsed() >= Duration::from_millis(250) {
            next_tick = Instant::now();
        }
    }
    Ok(())
}

async fn execute_cmd(model: &mut Model, runtime: &mut Runtime<FakePiAdapter>, cmd: Cmd) {
    match cmd {
        Cmd::Quit => model.running = false,
        Cmd::Submit(prompt) => match runtime.submit(prompt) {
            Ok(t) => {
                let id = t.id;
                model.tasks.push(t);
                model.selected = model.tasks.len() - 1;
                model.status = format!("queued {id}");
            }
            Err(e) => model.status = e.to_string(),
        },
        Cmd::Pause(id) => apply_runtime(model, runtime.pause(id)),
        Cmd::Resume(id) => apply_runtime(model, runtime.resume(id)),
        Cmd::Cancel(id) => apply_runtime(model, runtime.cancel(id)),
        Cmd::Retry(id) => {
            if let Some(t) = model.tasks.iter_mut().find(|t| t.id == id) {
                t.state = TaskState::Queued;
                t.failure_reason = None;
                match runtime.store.save_task(t) {
                    Ok(()) => model.status = "retry queued".into(),
                    Err(e) => model.status = format!("error: {e}"),
                }
            }
        }
        Cmd::Override(id) => {
            if let Some(t) = model.tasks.iter_mut().find(|t| t.id == id) {
                t.route.model = "manual-override".into();
                match runtime.store.save_task(t) {
                    Ok(()) => model.status = "deterministic override applied".into(),
                    Err(e) => model.status = format!("error: {e}"),
                }
            }
        }
        Cmd::None => {}
    }
}
fn apply_runtime(model: &mut Model, result: anyhow::Result<Task>) {
    match result {
        Ok(task) => {
            if let Some(current) = model.tasks.iter_mut().find(|t| t.id == task.id) {
                *current = task;
            }
            model.status = "runtime state updated".into();
        }
        Err(e) => model.status = format!("error: {e}"),
    }
}

pub fn view(f: &mut Frame<'_>, model: &Model) {
    let area = f.area();
    if area.width < 40 || area.height < 10 {
        f.render_widget(
            Paragraph::new(format!(
                "ASTER · {}\n{}\nTerminal too small (need 40×10)\nq quit",
                model.screen.title(),
                model.status
            ))
            .block(Block::bordered()),
            area,
        );
        return;
    }
    let compact = area.width < 80 || area.height < 20;
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .split(area);
    let tabs = Tabs::new(Screen::ALL.iter().map(|s| s.title()))
        .select(Screen::ALL.iter().position(|s| *s == model.screen).unwrap())
        .block(Block::bordered().title(" ASTER "))
        .highlight_style(Style::new().cyan().bold());
    f.render_widget(tabs, rows[0]);
    render_screen(f, model, rows[1], compact);
    f.render_widget(Paragraph::new(format!("> {}  │ {}  │ Tab screens · ↑↓ select · p pause · s resume · x cancel · r retry · o override · q quit",model.input,model.status)).block(Block::bordered()),rows[2]);
}
fn render_screen(f: &mut Frame<'_>, m: &Model, a: Rect, compact: bool) {
    let selected = m.tasks.get(m.selected);
    let text=match m.screen {
        Screen::Conversation => "Conversation\nType a task and press Enter. Runtime events arrive through subscriptions.".into(),
        Screen::Tasks => m.tasks.iter().enumerate().map(|(i,t)|format!("{} {:8} {:?} {}",if i==m.selected{"›"}else{" "},&t.id.to_string()[..8],t.state,t.prompt)).collect::<Vec<_>>().join("\n"),
        Screen::Dag => m.tasks.iter().map(|t|format!("{} ← {}",&t.id.to_string()[..8],if t.dependencies.is_empty(){"root".into()}else{t.dependencies.iter().map(|x|x.to_string()[..8].to_string()).collect::<Vec<_>>().join(",")})).collect::<Vec<_>>().join("\n"),
        Screen::Routing => selected.map(|t|format!("role: {}\nmodel: {}\neffort: {}\nrationale: {}",t.route.role,t.route.model,t.route.effort,t.route.rationale)).unwrap_or("No task".into()),
        Screen::Usage => selected.map(|t|format!("tokens: {} / {}",t.tokens_used,t.token_budget.map(|x|x.to_string()).unwrap_or("unlimited".into()))).unwrap_or("No usage".into()),
        Screen::Transcripts => selected.and_then(|t|t.output.clone()).unwrap_or("No transcript yet".into()),
        Screen::Audit => "Audit events are persisted by the runtime store.".into(), Screen::Approvals => "No pending approvals · permissions inherited from runtime policy.".into(),
        Screen::Context => "Context window · sources · compaction state".into(), Screen::Artifacts => selected.and_then(|t|t.verification.clone()).unwrap_or("No artifacts, diffs, or evidence".into()),
        Screen::Config => "Configuration is loaded from the existing config subsystem.".into(), Screen::Memory => "Memory index and retention status".into(),
        Screen::Providers => "Pi adapter · healthy · deterministic fake available".into(), Screen::Plugins => "No plugins registered".into(),
    };
    let title = if compact {
        format!(" {} · compact ", m.screen.title())
    } else {
        format!(" {} ", m.screen.title())
    };
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(title)),
        a,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Route;
    use ratatui::{Terminal, backend::TestBackend};
    fn render(w: u16, h: u16, m: &Model) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| view(f, m)).unwrap();
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    }
    #[test]
    fn update_navigation_and_controls_are_pure() {
        let mut m = Model::new(vec![sample()]);
        assert_eq!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(
                    KeyCode::Tab,
                    crossterm::event::KeyModifiers::NONE
                ))
            ),
            Cmd::None
        );
        assert_eq!(m.screen, Screen::Tasks);
        assert!(matches!(
            update(
                &mut m,
                Msg::Key(KeyEvent::new(
                    KeyCode::Char('p'),
                    crossterm::event::KeyModifiers::NONE
                ))
            ),
            Cmd::Pause(_)
        ));
    }
    #[test]
    fn render_wide_compact_and_degraded() {
        let m = Model::new(vec![sample()]);
        assert!(render(120, 30, &m).contains("Conversation"));
        assert!(render(60, 16, &m).contains("compact"));
        assert!(render(30, 8, &m).contains("too small"));
    }
    fn sample() -> Task {
        Task::new(
            "test task".into(),
            Route {
                role: "builder".into(),
                model: "fake".into(),
                effort: "low".into(),
                context_budget: 100,
                capabilities: vec![],
                isolation: vec![],
                verification: "test".into(),
                rationale: "deterministic".into(),
            },
        )
    }
}
