use crate::domain::Route;

#[derive(Default)]
pub struct Router;

impl Router {
    pub fn route(&self, prompt: &str) -> Route {
        let substantive = prompt.len() > 120
            || ["implement", "refactor", "review", "debug"]
                .iter()
                .any(|w| prompt.to_lowercase().contains(w));
        Route {
            role: if substantive {
                "implementer"
            } else {
                "orchestrator"
            }
            .into(),
            model: if substantive {
                "fake-terra"
            } else {
                "fake-luna"
            }
            .into(),
            effort: if substantive { "medium" } else { "low" }.into(),
            context_budget: if substantive { 16_000 } else { 4_000 },
            capabilities: vec!["workspace:read".into()],
            isolation: vec![
                "process:none".into(),
                "network:denied".into(),
                "credentials:none".into(),
            ],
            verification: "deterministic-output-check".into(),
            rationale: if substantive {
                "Task signals substantive repository work; use a capable low-cost route."
            } else {
                "Trivial request is handled directly without delegation."
            }
            .into(),
        }
    }
}
