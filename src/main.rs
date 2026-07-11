use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "aster", about = "Durable, observable agent harness")]
struct Cli {
    #[arg(long, default_value = ".aster/state.db")]
    state: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(parent) = cli.state.parent() {
        std::fs::create_dir_all(parent)?;
    }
    aster::tui::run(&cli.state).await
}
