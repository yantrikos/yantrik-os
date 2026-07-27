//! Adapter between the two `Embedder` traits.
//!
//! `yantrik_ml::traits::Embedder` and `yantrikdb_core::Embedder` are
//! structurally identical but are distinct traits. They used to be the same
//! one: the vendored fork of yantrikdb-core depended on yantrik-ml and reused
//! its definition. Upstream yantrikdb has no yantrik-ml dependency and defines
//! its own, so an ML embedder no longer satisfies the database's bound.
//!
//! The orphan rule forbids implementing the database's trait for an ML type
//! directly, so the handoff is a newtype owned by this crate — the one place
//! that depends on both.
//!
//! Yantrik OS keeps supplying its own Candle embedder rather than switching to
//! the bundled model2vec one: the embedding function determines vector
//! semantics, and changing it would invalidate every vector in an existing
//! `memory.db`. That is a migration, not a dependency bump.

/// Wraps an ML-side embedder so it satisfies `yantrikdb_core::Embedder`.
pub struct EmbedderBridge<E> {
    inner: E,
    identity: Option<String>,
}

impl<E> EmbedderBridge<E> {
    /// Bridge an embedder whose model identity is unknown.
    ///
    /// Upstream treats a `fingerprint() == None` embedder as
    /// `ExternalOrUnknown` provenance: it may attach to an empty or
    /// unknown-provenance database, but never to a populated `Known`-provenance
    /// one without a `reembed()`. Conservative-correct — an embedder that
    /// cannot prove its identity cannot prove it agrees with vectors already
    /// indexed.
    pub fn new(inner: E) -> Self {
        Self { inner, identity: None }
    }

    /// Bridge an embedder with a stable identity, so upstream can tell
    /// same-model-replacement (safe) from different-model-same-dim (silent
    /// corruption) when an embedder is swapped on a populated database.
    pub fn with_identity(inner: E, identity: impl Into<String>) -> Self {
        Self { inner, identity: Some(identity.into()) }
    }
}

impl<E> yantrikdb_core::Embedder for EmbedderBridge<E>
where
    E: yantrik_ml::traits::Embedder + Send + Sync,
{
    fn fingerprint(&self) -> Option<String> {
        self.identity.clone()
    }

    fn embed(
        &self,
        text: &str,
    ) -> std::result::Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.embed(text)
    }

    fn embed_batch(
        &self,
        texts: &[&str],
    ) -> std::result::Result<Vec<Vec<f32>>, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.embed_batch(texts)
    }

    fn dim(&self) -> usize {
        self.inner.dim()
    }
}
