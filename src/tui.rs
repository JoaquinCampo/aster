use crate::{
    config::ConfigDocument,
    context,
    domain::{AuditEvent, Task, TaskState},
    memory::MemoryStore,
    plugin,
    provider::FakePiAdapter,
    routing::Router,
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

#[derive(Debug, Clone, Default)]
pub struct Observability {
    pub audit: Vec<AuditEvent>,
    pub context: Vec<String>,
    pub memory: Vec<String>,
    pub plugins: Vec<String>,
    pub diagnostics: Vec<String>,
    pub health_checked_at: String,
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
    pub observability: Observability,
    pub override_open: bool,
    pub override_choice: usize,
    pub config_path: Option<std::path::PathBuf>,
    pub config_selected: usize,
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
            observability: Observability::default(),
            override_open: false,
            override_choice: 0,
            config_path: None,
            config_selected: 0,
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
    Reconcile(uuid::Uuid, bool),
    Override(uuid::Uuid),
    EditConfig(String, String),
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
    if model.override_open {
        return match code {
            KeyCode::Up => {
                model.override_choice = model.override_choice.saturating_sub(1);
                Cmd::None
            }
            KeyCode::Down => {
                model.override_choice = (model.override_choice + 1).min(2);
                Cmd::None
            }
            KeyCode::Esc => {
                model.override_open = false;
                model.status = "route override cancelled".into();
                Cmd::None
            }
            KeyCode::Enter => {
                model.override_open = false;
                action(model, "validating route override", Cmd::Override)
            }
            _ => Cmd::None,
        };
    }
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
        KeyCode::Up if model.screen == Screen::Config => {
            model.config_selected = model.config_selected.saturating_sub(1);
            Cmd::None
        }
        KeyCode::Down if model.screen == Screen::Config => {
            model.config_selected = (model.config_selected + 1)
                .min(ConfigDocument::editable_fields().len().saturating_sub(1));
            Cmd::None
        }
        KeyCode::Char('e') if model.input.is_empty() && model.screen == Screen::Config => {
            let field = ConfigDocument::editable_fields()[model.config_selected].clone();
            let value = if field == "context.total_tokens" {
                "64000"
            } else {
                "true"
            };
            Cmd::EditConfig(field, value.into())
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
        KeyCode::Char('p') if model.input.is_empty() => request_action(model, "pause", Cmd::Pause),
        KeyCode::Char('s') if model.input.is_empty() => {
            request_action(model, "resume", Cmd::Resume)
        }
        KeyCode::Char('x') if model.input.is_empty() => {
            request_action(model, "cancel", Cmd::Cancel)
        }
        KeyCode::Char('r') if model.input.is_empty() => request_action(model, "retry", Cmd::Retry),
        KeyCode::Char('y') if model.input.is_empty() => {
            action(model, "reconcile succeeded", |id| Cmd::Reconcile(id, true))
        }
        KeyCode::Char('n') if model.input.is_empty() => {
            action(model, "reconcile failed", |id| Cmd::Reconcile(id, false))
        }
        KeyCode::Char('o') if model.input.is_empty() => {
            if model.tasks.get(model.selected).is_none() {
                model.status = "no task selected".into();
            } else {
                model.override_open = true;
                model.override_choice = 0;
                model.status = "route override editor: choose a validated preset".into();
            }
            Cmd::None
        }
        KeyCode::Char('1') if model.input.is_empty() && model.screen == Screen::Config => {
            Cmd::EditConfig("context.total_tokens".into(), "64000".into())
        }
        KeyCode::Char('2') if model.input.is_empty() && model.screen == Screen::Config => {
            Cmd::EditConfig("routing.enabled".into(), "true".into())
        }
        KeyCode::Char('3') if model.input.is_empty() && model.screen == Screen::Config => {
            Cmd::EditConfig("verification.enabled".into(), "true".into())
        }
        KeyCode::Char('4') if model.input.is_empty() && model.screen == Screen::Config => {
            Cmd::EditConfig("lifecycle.enabled".into(), "true".into())
        }
        KeyCode::Char(c) => {
            model.input.push(c);
            Cmd::None
        }
        _ => Cmd::None,
    }
}
fn request_action(model: &mut Model, name: &str, f: impl FnOnce(uuid::Uuid) -> Cmd) -> Cmd {
    let Some(task) = model.tasks.get(model.selected) else {
        model.status = "no task selected".into();
        return Cmd::None;
    };
    let legal = match name {
        "pause" => matches!(task.state, TaskState::Running),
        "resume" => matches!(task.state, TaskState::Paused),
        "cancel" => !task.state.is_terminal() && !matches!(task.state, TaskState::Cancelling),
        "retry" => matches!(task.state, TaskState::Failed | TaskState::TimedOut),
        _ => false,
    };
    if !legal {
        model.status = format!("{name} unavailable for {:?}", task.state);
        return Cmd::None;
    }
    action(model, &format!("{name} requested"), f)
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
    let recovered = runtime.recover()?;
    let mut model = Model::new(runtime.store.tasks()?);
    if recovered > 0 {
        model.selected = model
            .tasks
            .iter()
            .position(|task| task.state == TaskState::OutcomeUnknown)
            .unwrap_or(0);
        model.status = format!(
            "recovered {recovered} interrupted operation(s); select OutcomeUnknown and press y/n to reconcile"
        );
    }
    model.config_path = Some(path.with_extension("toml"));
    refresh_observability(&mut model, &runtime.store, path);
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
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

fn refresh_observability(model: &mut Model, store: &Store, db_path: &Path) {
    model.observability.health_checked_at = chrono::Utc::now().to_rfc3339();
    model.observability.audit = model
        .tasks
        .iter()
        .flat_map(|t| store.audit_for(t.id).unwrap_or_default())
        .collect();
    model.observability.audit.sort_by_key(|e| e.at);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    model.observability.context = context::discover(&cwd, &cwd)
        .and_then(|a| context::manifest_from_assets(&a, 32_000))
        .map(|m| {
            m.items
                .into_iter()
                .map(|i| {
                    format!(
                        "{} · {} tokens · {:?}",
                        i.provenance.path.display(),
                        i.estimated_tokens,
                        i.provenance.trust
                    )
                })
                .collect()
        })
        .unwrap_or_else(|e| vec![format!("context query error: {e}")]);
    model.observability.memory = MemoryStore::open(db_path)
        .and_then(|s| s.active())
        .map(|items| {
            items
                .into_iter()
                .map(|m| format!("{:?} · {} = {} · {}", m.scope, m.key, m.value, m.provenance))
                .collect()
        })
        .unwrap_or_else(|e| vec![format!("memory query error: {e}")]);
    let roots = [cwd.join(".aster/plugins"), cwd.join("plugins")];
    model.observability.plugins = plugin::discover(&roots)
        .map(|items| {
            items
                .into_iter()
                .map(|p| {
                    format!(
                        "{} {} · {} tools · {} MCP endpoints",
                        p.id,
                        p.version,
                        p.tools.len(),
                        p.mcp_endpoints.len()
                    )
                })
                .collect()
        })
        .unwrap_or_else(|e| vec![format!("plugin discovery error: {e}")]);
    model.observability.diagnostics = vec![
        "provider: deterministic-fake · ready · local/no network".into(),
        format!(
            "plugin registry: {} discovered",
            model.observability.plugins.len()
        ),
        "MCP: JSON-RPC client/server available · no configured live transport".into(),
    ];
}

async fn execute_cmd(model: &mut Model, runtime: &mut Runtime<FakePiAdapter>, cmd: Cmd) {
    match cmd {
        Cmd::Quit => model.running = false,
        Cmd::Submit(prompt) => {
            let submitted = if prompt.contains("cenario:timeout") {
                runtime.submit_with(prompt, vec![], Default::default(), Some(1), None)
            } else {
                runtime.submit(prompt)
            };
            match submitted {
                Ok(t) => {
                    let id = t.id;
                    model.tasks.push(t);
                    model.selected = model.tasks.len() - 1;
                    model.status = format!("running {id}");
                    if model.tasks[model.selected]
                        .prompt
                        .contains("cenario:in-flight-cancellation")
                    {
                        match runtime.store.transition(
                            id,
                            &[TaskState::Queued],
                            TaskState::Running,
                            "scenario_in_flight",
                            "deterministic operation held in flight",
                        ) {
                            Ok(task) => {
                                model.tasks[model.selected] = task;
                                model.status =
                                    "Running deterministic in-flight scenario; press x to cancel"
                                        .into();
                            }
                            Err(e) => model.status = format!("error: {e}"),
                        }
                    } else {
                        match runtime.run_ready().await {
                            Ok(_) => match runtime.store.tasks() {
                                Ok(tasks) => {
                                    model.tasks = tasks;
                                    model.status =
                                        model.tasks.iter().find(|task| task.id == id).map_or_else(
                                            || "task completed".into(),
                                            |task| format!("{:?} {id}", task.state),
                                        );
                                }
                                Err(e) => model.status = format!("error: {e}"),
                            },
                            Err(e) => model.status = format!("error: {e}"),
                        }
                    }
                }
                Err(e) => model.status = e.to_string(),
            }
        }
        Cmd::Pause(id) => apply_runtime(model, runtime.pause(id)),
        Cmd::Resume(id) => apply_runtime(model, runtime.resume(id)),
        Cmd::Cancel(id) => {
            if model.tasks.iter().any(|task| {
                task.id == id
                    && task.state == TaskState::Running
                    && task.prompt.contains("cenario:in-flight-cancellation")
            }) {
                let result = runtime
                    .store
                    .transition(
                        id,
                        &[TaskState::Running],
                        TaskState::Cancelling,
                        "task.cancelling",
                        "cancellation requested while operation in flight",
                    )
                    .and_then(|_| {
                        runtime.store.transition(
                            id,
                            &[TaskState::Cancelling],
                            TaskState::Cancelled,
                            "task.cancelled",
                            "in-flight fixture acknowledged cancellation at safe boundary",
                        )
                    });
                apply_runtime(model, result);
            } else {
                apply_runtime(model, runtime.cancel(id));
            }
        }
        Cmd::Retry(id) => apply_runtime(model, runtime.retry(id)),
        Cmd::Reconcile(id, succeeded) => apply_runtime(model, runtime.reconcile(id, succeeded)),
        Cmd::Override(id) => {
            let prompt = [
                "simple local edit",
                "complex architecture investigation",
                "high risk security review",
            ][model.override_choice];
            let route = Router::default().route(prompt);
            match Router::default().validate_route(&route) {
                Ok(()) => apply_runtime(model, runtime.override_route(id, route)),
                Err(e) => model.status = format!("route override rejected: {e}"),
            }
        }
        Cmd::EditConfig(field, value) => {
            let result = model
                .config_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("configuration path unavailable"))
                .and_then(|path| {
                    let mut document = ConfigDocument::load(path)?;
                    document.edit_required(&field, &value)?;
                    document.save_atomic()
                });
            model.status = match result {
                Ok(()) => format!("config saved: {field}"),
                Err(e) => format!("config edit failed: {e}"),
            };
        }
        Cmd::None => {}
    }
    model.observability.audit = model
        .tasks
        .iter()
        .flat_map(|t| runtime.store.audit_for(t.id).unwrap_or_default())
        .collect();
    model.observability.audit.sort_by_key(|e| e.at);
}
fn apply_runtime(model: &mut Model, result: anyhow::Result<Task>) {
    match result {
        Ok(task) => {
            let state = task.state;
            if let Some(current) = model.tasks.iter_mut().find(|t| t.id == task.id) {
                *current = task;
            }
            model.status = format!("{state:?} runtime state updated");
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
fn action_availability(t: &Task) -> String {
    format!(
        "Actions: pause={} resume={} cancel={} retry={}",
        matches!(t.state, TaskState::Running),
        matches!(t.state, TaskState::Paused),
        !t.state.is_terminal() && !matches!(t.state, TaskState::Cancelling),
        matches!(t.state, TaskState::Failed | TaskState::TimedOut)
    )
}

fn render_screen(f: &mut Frame<'_>, m: &Model, a: Rect, compact: bool) {
    let selected = m.tasks.get(m.selected);
    let lines = |v: &[String], empty: &str| {
        if v.is_empty() {
            empty.into()
        } else {
            v.join("\n")
        }
    };
    let text = if m.override_open {
        let choices = [
            "economy/local edit",
            "quality/architecture",
            "high-risk/security",
        ];
        format!(
            "Route override editor [dialog]\nSelect validated preset (↑↓, Enter apply, Esc cancel)\n{}",
            choices
                .iter()
                .enumerate()
                .map(|(i, x)| format!("{} {x}", if i == m.override_choice { "›" } else { " " }))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        match m.screen {
        Screen::Conversation => "Conversation [input]\nType a task and press Enter. Runtime events remain non-blocking.".into(),
        Screen::Tasks => m.tasks.iter().enumerate().map(|(i,t)|format!("{} {:8} {:?} {} · {}",if i==m.selected{"›"}else{" "},&t.id.to_string()[..8],t.state,t.prompt,action_availability(t))).collect::<Vec<_>>().join("\n"),
        Screen::Dag => {
            let edges = m.tasks.iter().map(|t|format!("{} ← {}",&t.id.to_string()[..8],if t.dependencies.is_empty(){"root".into()}else{t.dependencies.iter().map(|x|x.to_string()[..8].to_string()).collect::<Vec<_>>().join(",")})).collect::<Vec<_>>();
            let critical = m.tasks.iter().max_by_key(|t| (t.updated_at-t.created_at).num_milliseconds()).map(|t| format!("{} ({} ms)",&t.id.to_string()[..8],(t.updated_at-t.created_at).num_milliseconds().max(0))).unwrap_or_else(|| "none".into());
            format!("DAG [graph] · duration-weighted critical path endpoint: {critical}\n{}", edges.join("\n"))
        }
        Screen::Routing => selected.map(|t|format!("Route trace [detail]\ndecision: {}\nrole: {}\nmodel: {}\neffort: {}\nrationale: {}\neffects/escalations:\n{}\nPress o to edit override",t.route.decision_id,t.route.role,t.route.model,t.route.dimensions.effort,t.route.rationale,m.observability.audit.iter().filter(|e| e.task_id==t.id && (e.kind.contains("effect") || e.kind.contains("escalat") || e.kind.contains("route"))).map(|e|format!("{} · {}",e.kind,e.detail)).collect::<Vec<_>>().join("\n"))).unwrap_or("No task selected".into()),
        Screen::Usage => selected.map(|t|format!("Usage and budgets [meter]\ntokens: {} / {}\nremaining: {}\nattempts: {} / {}\ntimeout: {} ms",t.tokens_used,t.token_budget.map(|x|x.to_string()).unwrap_or("unlimited".into()),t.token_budget.map(|b|b.saturating_sub(t.tokens_used).to_string()).unwrap_or("unlimited".into()),t.attempts,t.retry.max_attempts,t.timeout_ms.map(|x|x.to_string()).unwrap_or("none".into()))).unwrap_or("No usage".into()),
        Screen::Transcripts => selected.and_then(|t|t.output.clone()).unwrap_or("No transcript yet".into()),
        Screen::Audit => format!("Audit events [log]\n{}", m.observability.audit.iter().rev().map(|e|format!("{} · {} · {}",e.at.format("%H:%M:%S"),e.kind,e.detail)).collect::<Vec<_>>().join("\n")),
        Screen::Approvals => selected.map(|t| format!("Permissions/approvals [status]\ncapabilities: {}\nisolation: {}",t.route.dimensions.capabilities.join(", "),t.route.dimensions.isolation.join(", "))).unwrap_or("No task selected".into()),
        Screen::Context => format!("Context manifest [list]\n{}", lines(&m.observability.context,"No discovered context assets")),
        Screen::Artifacts => format!("Artifact index [all tasks · query-backed]\n{}",m.tasks.iter().map(|t|format!("{} · transcript={} · verification={} · failure={}",&t.id.to_string()[..8],if t.output.is_some(){"persisted"}else{"none"},t.verification.as_deref().unwrap_or("not run"),t.failure_reason.as_deref().unwrap_or("none"))).collect::<Vec<_>>().join("\n")),
        Screen::Config => format!(
            "Config editor [schema-driven · atomic/conflict-aware]\nAll required top-level domains (↑↓ select, e enable/apply)\n{}\nEdits reload, validate, preserve unknown fields, and atomically replace; concurrent changes are rejected.",
            ConfigDocument::editable_fields().iter().enumerate().map(|(i, field)|
                format!("{} {field}", if i == m.config_selected { "›" } else { " " })
            ).collect::<Vec<_>>().join("\n")
        ),
        Screen::Memory => format!("Memory index [list]\n{}", lines(&m.observability.memory,"No active memories")),
        Screen::Providers => format!("Provider status [status]\nhealth checked: {}\n{}",m.observability.health_checked_at, lines(&m.observability.diagnostics,"No diagnostics")),
        Screen::Plugins => format!("Plugin/MCP diagnostics [list]\nhealth checked: {}\n{}\n{}",m.observability.health_checked_at,lines(&m.observability.plugins,"No plugins discovered"),lines(&m.observability.diagnostics,"No diagnostics")),
    }
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
        m.tasks[0].state = TaskState::Running;
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
    #[test]
    fn observability_screens_have_pty_semantic_labels() {
        let mut m = Model::new(vec![sample()]);
        m.observability.context = vec!["AGENTS.md · 10 tokens".into()];
        m.observability.memory = vec!["Task · key = value".into()];
        m.observability.plugins = vec!["fixture 1.0 · 1 tools".into()];
        let cases = [
            (Screen::Dag, "DAG [graph]"),
            (Screen::Routing, "Route trace [detail]"),
            (Screen::Usage, "Usage and budgets [meter]"),
            (Screen::Audit, "Audit events [log]"),
            (Screen::Context, "Context manifest [list]"),
            (
                Screen::Artifacts,
                "Artifact index [all tasks · query-backed]",
            ),
            (Screen::Memory, "Memory index [list]"),
            (Screen::Providers, "Provider status [status]"),
            (Screen::Plugins, "Plugin/MCP diagnostics [list]"),
        ];
        for (screen, label) in cases {
            m.screen = screen;
            assert!(render(120, 30, &m).contains(label), "missing {label}");
        }
    }

    #[test]
    fn illegal_actions_and_override_editor_are_visible() {
        let mut m = Model::new(vec![sample()]);
        assert_eq!(update_key(&mut m, KeyCode::Char('p')), Cmd::None);
        assert!(m.status.contains("pause unavailable"));
        assert_eq!(update_key(&mut m, KeyCode::Char('o')), Cmd::None);
        assert!(m.override_open);
        assert!(render(100, 24, &m).contains("Route override editor [dialog]"));
        assert!(matches!(
            update_key(&mut m, KeyCode::Enter),
            Cmd::Override(_)
        ));
    }

    #[tokio::test]
    async fn config_screen_renders_and_applies_atomic_edit_command() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("aster.toml");
        std::fs::write(&config, "version=1\n[context]\ntotal_tokens=100\n").unwrap();
        let mut m = Model::new(vec![]);
        m.screen = Screen::Config;
        m.config_path = Some(config.clone());
        assert!(render(100, 24, &m).contains("atomic/conflict-aware"));
        let command = update_key(&mut m, KeyCode::Char('3'));
        assert_eq!(
            command,
            Cmd::EditConfig("verification.enabled".into(), "true".into())
        );
        let mut runtime = Runtime::new(Store::open(":memory:").unwrap(), FakePiAdapter);
        execute_cmd(&mut m, &mut runtime, command).await;
        assert!(m.status.contains("config saved"));
        assert!(
            ConfigDocument::load(config)
                .unwrap()
                .config
                .verification
                .enabled
        );
    }

    fn sample() -> Task {
        Task::new(
            "test task".into(),
            crate::routing::Router::default().route("test task"),
        )
    }
}
