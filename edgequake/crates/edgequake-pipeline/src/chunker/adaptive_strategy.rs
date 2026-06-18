//! Adaptive chunking strategy backed by the `adaptive_chunk` HTTP service (W1).
//!
//! @implements SoT W1 (§3.3, §3.4)
//!
//! # Responsibilities
//!
//! 1. Split incoming `content` on the modal markers `〈MODAL …〉…〈/MODAL〉`
//!    (U+3008 / U+3009 angle brackets). Each modal region becomes **one atomic**
//!    [`ChunkResult`] — markers included — and is **never** sent to the chunking
//!    service (modal-span atomicity is owned entirely by this strategy; SoT §3.3-2).
//! 2. Text gaps between modal regions are POSTed to `{base_url}/chunk` and the
//!    response `chunks[]` are mapped to [`ChunkResult`].
//! 3. Segments are interleaved in document order and assigned sequential
//!    `chunk_order_index`.
//!
//! The producer (W2 `modal.py`) and this consumer MUST use byte-identical markers.
//! The marker constants below are the single source of truth on the Rust side.

use async_trait::async_trait;
use serde::Deserialize;

use super::text_utils::estimate_tokens;
use super::types::{ChunkResult, ChunkerConfig, ChunkingStrategy};
use crate::error::{PipelineError, Result};

/// Opening angle bracket of a modal marker (U+3008, 〈).
pub const MODAL_ANGLE_OPEN: char = '\u{3008}';
/// Closing angle bracket of a modal marker (U+3009, 〉).
pub const MODAL_ANGLE_CLOSE: char = '\u{3009}';

/// Start of an opening modal marker: `〈MODAL` (everything up to the attributes).
const MODAL_OPEN_PREFIX: &str = "\u{3008}MODAL";
/// The complete closing modal marker: `〈/MODAL〉`.
const MODAL_CLOSE: &str = "\u{3008}/MODAL\u{3009}";

/// Default base URL for the `adaptive_chunk` service.
const DEFAULT_ADAPTIVE_CHUNK_URL: &str = "http://localhost:18060";
/// Environment variable overriding the `adaptive_chunk` base URL.
const ADAPTIVE_CHUNK_URL_ENV: &str = "ADAPTIVE_CHUNK_URL";

/// An ordered slice of the document: either a modal region or a plain-text gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A `〈MODAL …〉…〈/MODAL〉` region, including its markers. Emitted atomically.
    Modal(String),
    /// Plain text between modal regions. Delegated to the chunking service.
    Text(String),
}

/// Split `content` into ordered (modal | text) segments.
///
/// Pure: no HTTP, no allocation beyond the returned `String`s. This is the
/// unit-tested core of the strategy.
///
/// Matching is deliberately permissive about the marker body: an open marker is
/// `〈MODAL` … up to the next `〉`, and the region ends at the first following
/// `〈/MODAL〉`. A malformed open marker with no matching close is treated as
/// plain text (so we never silently swallow document content).
pub fn split_segments(content: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;

    while cursor < content.len() {
        // Find the next opening marker prefix at or after the cursor.
        let open_rel = match content[cursor..].find(MODAL_OPEN_PREFIX) {
            Some(idx) => idx,
            None => break,
        };
        let open_abs = cursor + open_rel;

        // The opening marker must terminate with a closing angle bracket `〉`.
        let after_prefix = open_abs + MODAL_OPEN_PREFIX.len();
        let open_end_rel = match content[after_prefix..].find(MODAL_ANGLE_CLOSE) {
            Some(idx) => idx,
            None => break, // No `〉` closing the open marker -> rest is text.
        };
        let open_marker_end = after_prefix + open_end_rel + MODAL_ANGLE_CLOSE.len_utf8();

        // The region must be closed by `〈/MODAL〉`.
        let close_rel = match content[open_marker_end..].find(MODAL_CLOSE) {
            Some(idx) => idx,
            None => break, // Unterminated region -> treat remainder as text.
        };
        let region_end = open_marker_end + close_rel + MODAL_CLOSE.len();

        // Emit any preceding text gap.
        if open_abs > cursor {
            push_text(&mut segments, &content[cursor..open_abs]);
        }

        // Emit the whole modal region (markers included) as one atomic segment.
        segments.push(Segment::Modal(content[open_abs..region_end].to_string()));
        cursor = region_end;
    }

    // Trailing text after the last modal region (or the whole doc if no modals).
    if cursor < content.len() {
        push_text(&mut segments, &content[cursor..]);
    }

    segments
}

/// Push a text segment, skipping content that is empty or whitespace-only.
fn push_text(segments: &mut Vec<Segment>, text: &str) {
    if !text.trim().is_empty() {
        segments.push(Segment::Text(text.to_string()));
    }
}

/// Request body for `POST /chunk` (matches `service/schemas.py::ChunkRequest`).
#[derive(serde::Serialize)]
struct ChunkServiceRequest<'a> {
    text: &'a str,
    doc_name: &'a str,
}

/// One chunk in the `/chunk` response (matches `runner.py::_chunk_to_dict`).
#[derive(Debug, Deserialize)]
struct ServiceChunk {
    chunk_text: String,
    /// Token count from the normalized tokenizer (D4). May be absent on error.
    #[serde(default)]
    chunk_len: usize,
}

/// `/chunk` response envelope (we only need `chunks[]`).
#[derive(Debug, Deserialize)]
struct ChunkServiceResponse {
    chunks: Vec<ServiceChunk>,
}

/// Chunking strategy that delegates text gaps to the `adaptive_chunk` service
/// while keeping modal regions atomic.
pub struct AdaptiveChunkStrategy {
    base_url: String,
    client: reqwest::Client,
}

impl AdaptiveChunkStrategy {
    /// Create a strategy targeting `base_url` (trailing slash trimmed).
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Create a strategy reading `ADAPTIVE_CHUNK_URL`
    /// (default `http://localhost:18060`).
    pub fn from_env() -> Self {
        let base_url = std::env::var(ADAPTIVE_CHUNK_URL_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ADAPTIVE_CHUNK_URL.to_string());
        Self::new(base_url)
    }

    /// POST a text gap to `/chunk` and return its chunks (content + tokens).
    async fn chunk_text_gap(&self, text: &str) -> Result<Vec<(String, usize)>> {
        let url = format!("{}/chunk", self.base_url);
        let body = ChunkServiceRequest {
            text,
            doc_name: "",
        };

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                PipelineError::ChunkingError(format!(
                    "adaptive_chunk request to {url} failed: {e}"
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(PipelineError::ChunkingError(format!(
                "adaptive_chunk {url} returned HTTP {status}: {detail}"
            )));
        }

        let parsed: ChunkServiceResponse = response.json().await.map_err(|e| {
            PipelineError::ChunkingError(format!(
                "adaptive_chunk {url} response was not valid JSON: {e}"
            ))
        })?;

        Ok(parsed
            .chunks
            .into_iter()
            .map(|c| {
                let tokens = if c.chunk_len > 0 {
                    c.chunk_len
                } else {
                    estimate_tokens(&c.chunk_text)
                };
                (c.chunk_text, tokens)
            })
            .collect())
    }
}

#[async_trait]
impl ChunkingStrategy for AdaptiveChunkStrategy {
    async fn chunk(&self, content: &str, _config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        let segments = split_segments(content);

        let mut results: Vec<ChunkResult> = Vec::new();
        let mut order: usize = 0;

        for segment in segments {
            match segment {
                Segment::Modal(region) => {
                    // Atomic: the whole region (markers included) is one chunk.
                    let tokens = estimate_tokens(&region);
                    results.push(ChunkResult {
                        content: region,
                        tokens,
                        chunk_order_index: order,
                    });
                    order += 1;
                }
                Segment::Text(text) => {
                    for (chunk_text, tokens) in self.chunk_text_gap(&text).await? {
                        // Skip empty chunks the service may emit defensively.
                        if chunk_text.trim().is_empty() {
                            continue;
                        }
                        results.push(ChunkResult {
                            content: chunk_text,
                            tokens,
                            chunk_order_index: order,
                        });
                        order += 1;
                    }
                }
            }
        }

        Ok(results)
    }

    fn name(&self) -> &str {
        "adaptive_chunk"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modal(id: &str, ty: &str, desc: &str, payload: &str) -> String {
        format!(
            "{MODAL_OPEN_PREFIX} id=\"{id}\" type=\"{ty}\"{close}{desc}\n{payload}{close_marker}",
            close = MODAL_ANGLE_CLOSE,
            close_marker = MODAL_CLOSE,
        )
    }

    #[test]
    fn plain_text_only_is_single_text_segment() {
        let segs = split_segments("just some plain prose with no modals");
        assert_eq!(
            segs,
            vec![Segment::Text(
                "just some plain prose with no modals".to_string()
            )]
        );
    }

    #[test]
    fn empty_or_whitespace_yields_no_segments() {
        assert!(split_segments("").is_empty());
        assert!(split_segments("   \n\t  ").is_empty());
    }

    #[test]
    fn single_modal_region_is_atomic_and_includes_markers() {
        let region = modal("T1", "table", "A sales table", "<table>...</table>");
        let segs = split_segments(&region);
        assert_eq!(segs, vec![Segment::Modal(region)]);
    }

    #[test]
    fn interleaves_text_and_modal_in_document_order() {
        let m1 = modal("T1", "table", "desc1", "payload1");
        let m2 = modal("E1", "equation", "desc2", "E=mc^2");
        let content = format!("intro text {m1} middle text {m2} tail text");

        let segs = split_segments(&content);
        assert_eq!(
            segs,
            vec![
                Segment::Text("intro text ".to_string()),
                Segment::Modal(m1.clone()),
                Segment::Text(" middle text ".to_string()),
                Segment::Modal(m2.clone()),
                Segment::Text(" tail text".to_string()),
            ]
        );
    }

    #[test]
    fn modal_with_no_surrounding_text() {
        let m1 = modal("I1", "image", "a chart", "img:/path.png");
        let m2 = modal("T2", "table", "rows", "<table/>");
        let content = format!("{m1}{m2}");
        let segs = split_segments(&content);
        assert_eq!(segs, vec![Segment::Modal(m1), Segment::Modal(m2)]);
    }

    #[test]
    fn payload_containing_angle_brackets_does_not_break_region() {
        // HTML payload may contain '<' '>' but NOT the CJK angle brackets,
        // so the region boundary detection is unaffected.
        let region = modal("T1", "table", "desc", "<tr><td>1 &lt; 2</td></tr>");
        let content = format!("before {region} after");
        let segs = split_segments(&content);
        assert_eq!(
            segs,
            vec![
                Segment::Text("before ".to_string()),
                Segment::Modal(region),
                Segment::Text(" after".to_string()),
            ]
        );
    }

    #[test]
    fn unterminated_open_marker_is_treated_as_text() {
        // `〈MODAL ...〉` with no closing `〈/MODAL〉` must not swallow the doc.
        let content = format!(
            "text {MODAL_OPEN_PREFIX} id=\"X\" type=\"table\"{} body with no close",
            MODAL_ANGLE_CLOSE
        );
        let segs = split_segments(&content);
        assert_eq!(segs, vec![Segment::Text(content)]);
    }

    #[test]
    fn open_prefix_without_closing_angle_is_text() {
        let content = format!("text {MODAL_OPEN_PREFIX} id=\"X\" no closing angle bracket here");
        let segs = split_segments(&content);
        assert_eq!(segs, vec![Segment::Text(content)]);
    }

    #[test]
    fn from_env_defaults_when_unset() {
        // Save/restore to avoid cross-test interference.
        let prev = std::env::var(ADAPTIVE_CHUNK_URL_ENV).ok();
        std::env::remove_var(ADAPTIVE_CHUNK_URL_ENV);
        let s = AdaptiveChunkStrategy::from_env();
        assert_eq!(s.base_url, DEFAULT_ADAPTIVE_CHUNK_URL);
        if let Some(v) = prev {
            std::env::set_var(ADAPTIVE_CHUNK_URL_ENV, v);
        }
    }

    #[test]
    fn new_trims_trailing_slash() {
        let s = AdaptiveChunkStrategy::new("http://example.com:18060/");
        assert_eq!(s.base_url, "http://example.com:18060");
    }
}
