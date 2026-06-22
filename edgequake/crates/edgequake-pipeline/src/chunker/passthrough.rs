//! Passthrough chunking strategy for the kb-pipeline facade (SoT §6).
//!
//! The kb-pipeline facade performs chunking upstream (via `adaptive_chunk`) and
//! hands edgequake a single pre-chunked payload whose chunk boundaries are the
//! ASCII **RECORD SEPARATOR** control character `U+001E`. This strategy simply
//! splits on that separator so edgequake stores exactly the chunks the facade
//! chose — no re-chunking, no external calls, no token-size heuristics.
//!
//! Contract (matches the facade `\u{001e}` join in `service/edgequake.py`):
//! - Split `content` on `U+001E`.
//! - Each non-empty fragment becomes one [`ChunkResult`] in document order with
//!   a sequential `chunk_order_index` (0, 1, 2, ...).
//! - Empty fragments (produced by leading/trailing/double separators) are
//!   skipped — they never become chunks.
//! - Content with no separator yields exactly one chunk.

use async_trait::async_trait;

use super::text_utils::estimate_tokens;
use super::types::{ChunkResult, ChunkerConfig, ChunkingStrategy};
use crate::error::Result;

/// ASCII RECORD SEPARATOR (`U+001E`) — the chunk boundary the kb-pipeline
/// facade uses when joining pre-chunked content for edgequake.
pub const RECORD_SEPARATOR: char = '\u{1e}';

/// Chunking strategy that splits pre-chunked facade payloads on `U+001E`.
///
/// Zero-cost, side-effect-free: no external calls, no token-size re-chunking.
/// edgequake stores exactly the chunks the facade chose.
pub struct PassthroughStrategy;

#[async_trait]
impl ChunkingStrategy for PassthroughStrategy {
    async fn chunk(&self, content: &str, _config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        let mut results: Vec<ChunkResult> = Vec::new();
        let mut order: usize = 0;

        for fragment in content.split(RECORD_SEPARATOR) {
            // Empty fragments (leading/trailing/doubled separators) never
            // become chunks; surviving chunks keep contiguous order indices.
            if fragment.is_empty() {
                continue;
            }
            results.push(ChunkResult {
                content: fragment.to_string(),
                tokens: estimate_tokens(fragment),
                chunk_order_index: order,
            });
            order += 1;
        }

        Ok(results)
    }

    fn name(&self) -> &str {
        "passthrough"
    }
}

#[cfg(test)]
mod tests {
    use super::PassthroughStrategy;
    use crate::chunker::{ChunkerConfig, ChunkingStrategy};

    /// The U+001E record separator the facade uses to join chunks.
    const RS: char = '\u{1e}';

    async fn chunk(content: &str) -> Vec<(String, usize)> {
        let strategy = PassthroughStrategy;
        let config = ChunkerConfig::default();
        strategy
            .chunk(content, &config)
            .await
            .expect("passthrough chunking should not fail")
            .into_iter()
            .map(|r| (r.content, r.chunk_order_index))
            .collect()
    }

    #[tokio::test]
    async fn splits_on_record_separator_preserving_order() {
        let content = format!("a{RS}b{RS}c");
        let chunks = chunk(&content).await;
        assert_eq!(
            chunks,
            vec![
                ("a".to_string(), 0),
                ("b".to_string(), 1),
                ("c".to_string(), 2),
            ]
        );
    }

    #[tokio::test]
    async fn single_fragment_without_separator_is_one_chunk() {
        let chunks = chunk("only one chunk here").await;
        assert_eq!(chunks, vec![("only one chunk here".to_string(), 0)]);
    }

    #[tokio::test]
    async fn empty_fragments_are_skipped_and_order_is_contiguous() {
        // Leading, trailing, and doubled separators produce empty fragments
        // that must NOT become chunks; surviving chunks get contiguous indices.
        let content = format!("{RS}a{RS}{RS}b{RS}");
        let chunks = chunk(&content).await;
        assert_eq!(
            chunks,
            vec![("a".to_string(), 0), ("b".to_string(), 1)]
        );
    }

    #[tokio::test]
    async fn all_empty_yields_no_chunks() {
        let content = format!("{RS}{RS}");
        let chunks = chunk(&content).await;
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn strategy_name_is_passthrough() {
        assert_eq!(PassthroughStrategy.name(), "passthrough");
    }
}
