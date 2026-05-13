use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[command(name = "lazyme")]
pub struct Args {
    /// Remote name to fetch from (global default for all targets)
    #[arg(short = 'R', long, default_value = "origin")]
    pub remote: String,

    /// Poll interval in seconds (global default for all targets)
    #[arg(short, long, default_value_t = 60)]
    pub interval: u64,

    /// Port for the web UI
    #[arg(short = 'p', long, default_value_t = 8080)]
    pub port: u16,

    /// One or more target names to watch (default: all from targets.toml)
    pub filter: Vec<String>,
}
