//! Document-scoped graph handler (`GET /api/v1/graph/documents/{document_id}`).
//!
//! Returns the knowledge subgraph (entities + relationships) extracted from a
//! single document, in the same `KnowledgeGraphResponse` shape the frontend
//! `GraphRenderer` consumes. Reuses the bounded document-scoped scan helpers
//! (`find_document_nodes` / `find_document_edges`, SPEC-006 P1) so it never
//! loads the full workspace graph.

use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Path, State},
    Json,
};
use tracing::{debug, warn};

use crate::error::ApiResult;
use crate::handlers::graph_types::*;
use crate::handlers::isolation::verify_document_access;
use crate::middleware::TenantContext;
use crate::services::{
    admit_graph_materialization, find_document_edges, find_document_nodes, DocumentSourceScope,
};
use crate::state::AppState;

/// Get the knowledge subgraph for a single document.
///
/// # Implements
///
/// - **FEAT0601**: Knowledge Graph Visualization (document-scoped)
///
/// # Enforces
///
/// - **BR0201**: Tenant isolation (filters by workspace)
/// - **SPEC-006 P1**: Bounded document-scoped scan (no full-graph load)
#[utoipa::path(
    get,
    path = "/api/v1/graph/documents/{document_id}",
    tag = "Graph",
    params(
        ("document_id" = String, Path, description = "Document ID to scope the graph to")
    ),
    responses(
        (status = 200, description = "Document-scoped graph", body = KnowledgeGraphResponse),
        (status = 404, description = "Document not found")
    )
)]
pub async fn get_document_graph(
    State(state): State<AppState>,
    tenant_ctx: TenantContext,
    Path(document_id): Path<String>,
) -> ApiResult<Json<KnowledgeGraphResponse>> {
    let request_start = std::time::Instant::now();

    // SECURITY: Enforce strict tenant context requirement (mirrors get_graph).
    if tenant_ctx.tenant_id.is_none() || tenant_ctx.workspace_id.is_none() {
        warn!(
            tenant_id = ?tenant_ctx.tenant_id,
            workspace_id = ?tenant_ctx.workspace_id,
            "Tenant context missing - returning empty document graph for security"
        );
        return Ok(Json(KnowledgeGraphResponse {
            nodes: vec![],
            edges: vec![],
            is_truncated: false,
            total_nodes: 0,
            total_edges: 0,
        }));
    }

    // SECURITY: Verify the document belongs to the requesting tenant/workspace.
    verify_document_access(state.storage.kv_storage.as_ref(), &document_id, &tenant_ctx).await?;

    let _materialize_guard = admit_graph_materialization(&state)?;

    let scope = DocumentSourceScope::from_document_id(document_id.clone());
    let raw_nodes = find_document_nodes(&state.storage.graph_storage, Some(&tenant_ctx), &scope)
        .await?;
    let raw_edges = find_document_edges(&state.storage.graph_storage, Some(&tenant_ctx), &scope)
        .await?;

    // Degree is not provided by the scan helpers; compute it from the
    // document-scoped edge set so node sizing works in the renderer.
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for e in &raw_edges {
        *degree.entry(e.source.as_str()).or_default() += 1;
        *degree.entry(e.target.as_str()).or_default() += 1;
    }

    let nodes: Vec<GraphNodeResponse> = raw_nodes
        .iter()
        .map(|n| GraphNodeResponse {
            id: n.id.clone(),
            label: n.id.clone(),
            node_type: n
                .properties
                .get("entity_type")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string(),
            description: n
                .properties
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            degree: degree.get(n.id.as_str()).copied().unwrap_or(0),
            properties: serde_json::to_value(&n.properties).unwrap_or_default(),
        })
        .collect();

    // Defense-in-depth: only keep edges whose endpoints are both in the
    // returned node set (mirrors get_graph).
    let node_ids: HashSet<&String> = nodes.iter().map(|n| &n.id).collect();
    let edges: Vec<GraphEdgeResponse> = raw_edges
        .into_iter()
        .filter(|e| node_ids.contains(&e.source) && node_ids.contains(&e.target))
        .map(|e| GraphEdgeResponse {
            source: e.source,
            target: e.target,
            relationship_type: e
                .properties
                .get("relation_type")
                .and_then(|v| v.as_str())
                .unwrap_or("RELATED_TO")
                .to_string(),
            weight: e
                .properties
                .get("weight")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0) as f32,
            properties: serde_json::to_value(&e.properties).unwrap_or_default(),
        })
        .collect();

    let total_nodes = nodes.len();
    let total_edges = edges.len();

    debug!(
        document_id = %document_id,
        elapsed_ms = request_start.elapsed().as_millis(),
        total_nodes,
        total_edges,
        "Document graph query completed"
    );

    Ok(Json(KnowledgeGraphResponse {
        nodes,
        edges,
        is_truncated: false,
        total_nodes,
        total_edges,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};

    #[tokio::test]
    async fn test_get_document_graph_empty_tenant_returns_empty() {
        let state = AppState::test_state();
        // No tenant/workspace context -> strict gate returns an empty graph.
        let tenant_ctx = TenantContext::default();

        let result = get_document_graph(
            State(state),
            tenant_ctx,
            Path("doc-does-not-exist".to_string()),
        )
        .await;

        let response = result.expect("empty-tenant path returns Ok").0;
        assert!(response.nodes.is_empty());
        assert!(response.edges.is_empty());
        assert_eq!(response.total_nodes, 0);
        assert_eq!(response.total_edges, 0);
    }
}
