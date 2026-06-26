"use client";

/**
 * Standalone per-document graph popup.
 *
 * Opened in a new window by external apps (e.g. the 지식베이스 콘솔 document
 * detail "그래프 보기" button). Unlike the dashboard graph view, this page does
 * NOT depend on the workspace selector / auth gate: it reads the document id,
 * workspace id and (optional) tenant id straight from the URL query and calls
 * the edgequake graph API with explicit X-Tenant-ID / X-Workspace-ID headers.
 *
 *   /popup/graph?document=<docId>&workspace=<wsId>[&tenant=<tenantId>]
 *
 * If `tenant` is omitted it falls back to the default tenant (slug "default",
 * else the first tenant) — same rule the kb-backend uses.
 */

import { Suspense, useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { Loader2 } from "lucide-react";

import { GraphRenderer } from "@/components/graph/graph-renderer";
import { useGraphStore } from "@/stores/use-graph-store";
import { getRuntimeApiBaseUrl } from "@/lib/runtime-config";
import type { GraphEdge, GraphNode, KnowledgeGraph } from "@/types/graph";

async function resolveTenantId(base: string, provided: string | null): Promise<string | null> {
  if (provided) return provided;
  try {
    const res = await fetch(`${base}/tenants`, { headers: { Accept: "application/json" } });
    if (!res.ok) return null;
    const data = await res.json();
    const items: Array<{ id?: string; slug?: string }> = data?.items ?? data ?? [];
    const def = items.find((t) => t.slug === "default") ?? items[0];
    return def?.id ?? null;
  } catch {
    return null;
  }
}

function PopupGraph() {
  const params = useSearchParams();
  const documentId = params.get("document");
  const workspaceId = params.get("workspace");
  const tenantParam = params.get("tenant");

  const setColorMode = useGraphStore((s) => s.setColorMode);
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [status, setStatus] = useState<"loading" | "error" | "empty" | "ready">("loading");
  const [message, setMessage] = useState<string>("");

  // Color nodes by community (Louvain runs inside GraphRenderer).
  useEffect(() => {
    const prev = useGraphStore.getState().colorMode;
    setColorMode("community");
    return () => setColorMode(prev);
  }, [setColorMode]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!documentId || !workspaceId) {
        setStatus("error");
        setMessage("document/workspace 파라미터가 필요합니다.");
        return;
      }
      const base = getRuntimeApiBaseUrl();
      const tenantId = await resolveTenantId(base, tenantParam);
      if (!tenantId) {
        if (!cancelled) { setStatus("error"); setMessage("tenant 를 확인할 수 없습니다."); }
        return;
      }
      try {
        const res = await fetch(`${base}/graph/documents/${encodeURIComponent(documentId)}`, {
          headers: {
            Accept: "application/json",
            "X-Tenant-ID": tenantId,
            "X-Workspace-ID": workspaceId,
          },
        });
        if (!res.ok) {
          if (!cancelled) { setStatus("error"); setMessage(`그래프를 불러오지 못했습니다 (HTTP ${res.status}).`); }
          return;
        }
        const data: KnowledgeGraph = await res.json();
        // Normalize legacy edge_type → relationship_type (mirror getGraph).
        if (data.edges) {
          data.edges = data.edges.map((e: GraphEdge) => ({
            ...e,
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            relationship_type: e.relationship_type || (e as any).edge_type || "",
          }));
        }
        if (cancelled) return;
        if (!data.nodes || data.nodes.length === 0) { setStatus("empty"); return; }
        setGraph(data);
        setStatus("ready");
      } catch {
        if (!cancelled) { setStatus("error"); setMessage("그래프 요청 중 오류가 발생했습니다."); }
      }
    })();
    return () => { cancelled = true; };
  }, [documentId, workspaceId, tenantParam]);

  if (status === "loading") {
    return (
      <div className="flex h-screen w-screen items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }
  if (status === "error") {
    return (
      <div className="flex h-screen w-screen items-center justify-center p-6 text-center text-sm text-destructive">
        {message}
      </div>
    );
  }
  if (status === "empty") {
    return (
      <div className="flex h-screen w-screen items-center justify-center p-6 text-center text-sm text-muted-foreground">
        이 문서에서 추출된 그래프가 없습니다.
      </div>
    );
  }

  const nodes: GraphNode[] = graph?.nodes ?? [];
  const edges: GraphEdge[] = graph?.edges ?? [];
  return (
    <div className="h-screen w-screen">
      <GraphRenderer nodes={nodes} edges={edges} />
    </div>
  );
}

export default function PopupGraphPage() {
  return (
    <Suspense
      fallback={
        <div className="flex h-screen w-screen items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
        </div>
      }
    >
      <PopupGraph />
    </Suspense>
  );
}
