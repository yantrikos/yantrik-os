mod server;

pub use server::YantrikMcpServer;

use std::path::Path;

/// Initialize YantrikDB with an embedder and start the MCP stdio server.
pub async fn run_server(
    db_path: &str,
    embedding_dim: usize,
    embedder_dir: Option<&Path>,
) -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};
    use yantrik_ml::CandleEmbedder;

    // Load embedder
    let embedder = if let Some(dir) = embedder_dir {
        tracing::info!(dir = %dir.display(), "Loading embedder from directory");
        CandleEmbedder::from_dir(dir)?
    } else {
        tracing::info!("Downloading MiniLM embedder from HuggingFace Hub");
        CandleEmbedder::from_hub("sentence-transformers/all-MiniLM-L6-v2", None)?
    };

    // Create YantrikDB
    let mut db = yantrikdb_core::YantrikDB::new(db_path, embedding_dim)?;
    db.set_embedder(Box::new(embedder));

    let stats = db.stats(None)?;
    tracing::info!(
        active = stats.active_memories,
        entities = stats.entities,
        edges = stats.edges,
        "YantrikDB initialized"
    );

    // Start MCP server over stdio
    let server = YantrikMcpServer::new(db);
    let service = server.serve(stdio()).await.map_err(|e| {
        anyhow::anyhow!("MCP server error: {e}")
    })?;

    tracing::info!("YantrikDB MCP server running on stdio");
    service.waiting().await?;

    Ok(())
}
