use std::path::PathBuf;
use clap::Parser;

#[derive(Parser)]
#[command(name = "yantrik-mcp", about = "YantrikDB MCP server — cognitive memory for AI assistants")]
struct Cli {
    /// Path to the YantrikDB database file.
    #[arg(long, default_value = "memory.db")]
    db: String,

    /// Embedding dimension (must match the embedder model).
    #[arg(long, default_value_t = 384)]
    embedding_dim: usize,

    /// Path to a pre-downloaded embedder model directory.
    /// If omitted, downloads all-MiniLM-L6-v2 from HuggingFace on first run.
    #[arg(long)]
    embedder_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // MCP stdio uses stdout for JSON-RPC, so logs must go to stderr
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cli = Cli::parse();

    yantrik_mcp::run_server(
        &cli.db,
        cli.embedding_dim,
        cli.embedder_dir.as_deref(),
    ).await
}
