use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Clone, Debug)]
#[command(name = "deployd")]
pub struct Args {
    /// Path to the git repository to watch
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Remote name to fetch from
    #[arg(short = 'R', long, default_value = "origin")]
    pub remote: String,

    /// Branch to watch (default: main, overridden by .deployd/config.toml)
    #[arg(short, long, default_value = "main")]
    pub branch: String,

    /// Poll interval in seconds
    #[arg(short, long, default_value_t = 60)]
    pub interval: u64,

    /// Build command (default: "cargo build --release", overridden by config)
    #[arg(short = 'B', long, default_value = "cargo build --release")]
    pub build: String,

    /// Path to the built artifact, relative to repo root
    #[arg(short = 'a', long)]
    pub artifact: Option<PathBuf>,

    /// Command to run the app after deploy (optional)
    #[arg(short = 'x', long)]
    pub run: Option<String>,

    /// Profile name in .deployd/ (loads config.{profile}.toml)
    #[arg(long)]
    pub profile: Option<String>,

    /// Port for the web UI
    #[arg(short = 'p', long, default_value_t = 8080)]
    pub port: u16,
}
