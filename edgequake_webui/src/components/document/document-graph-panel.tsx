"use client";

/**
 * DocumentGraphPanel — lazy, per-document knowledge-graph view.
 *
 * Renders the subgraph (entities + relationships) extracted from a single
 * document, colored by community. Lazy by design: nothing is fetched or
 * rendered until the user clicks "그래프 보기" — opening a document never pays
 * the graph fetch / Louvain / Sigma cost unless explicitly requested.
 */

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Network, Loader2, ChevronUp } from "lucide-react";

import { getDocumentGraph } from "@/lib/api/edgequake";
import { GraphRenderer } from "@/components/graph/graph-renderer";
import { useGraphStore } from "@/stores/use-graph-store";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

export function DocumentGraphPanel({ documentId }: { documentId: string }) {
  const [show, setShow] = useState(false);
  const setColorMode = useGraphStore((s) => s.setColorMode);

  // Force community coloring while this panel is mounted, then restore the
  // user's previous colorMode so the global graph page is not affected.
  useEffect(() => {
    if (!show) return;
    const previous = useGraphStore.getState().colorMode;
    setColorMode("community");
    return () => {
      setColorMode(previous);
    };
  }, [show, setColorMode]);

  const { data, isLoading, isError } = useQuery({
    queryKey: ["document-graph", documentId],
    queryFn: () => getDocumentGraph(documentId),
    enabled: show && !!documentId,
    staleTime: 60_000,
  });

  if (!show) {
    return (
      <Button variant="outline" size="sm" onClick={() => setShow(true)}>
        <Network className="h-4 w-4 mr-2" />
        그래프 보기
      </Button>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-end">
        <Button variant="ghost" size="sm" onClick={() => setShow(false)}>
          <ChevronUp className="h-4 w-4 mr-1" />
          접기
        </Button>
      </div>

      {isLoading ? (
        <div className="flex h-[480px] w-full items-center justify-center rounded-md border">
          <div className="flex flex-col items-center gap-2 text-muted-foreground">
            <Loader2 className="h-6 w-6 animate-spin" />
            <Skeleton className="h-3 w-32" />
          </div>
        </div>
      ) : isError ? (
        <div className="flex h-[480px] w-full items-center justify-center rounded-md border text-sm text-destructive">
          그래프를 불러오지 못했습니다.
        </div>
      ) : !data || data.nodes.length === 0 ? (
        <div className="flex h-[480px] w-full items-center justify-center rounded-md border text-sm text-muted-foreground">
          이 문서에서 추출된 그래프가 없습니다.
        </div>
      ) : (
        <div className="h-[480px] w-full rounded-md border overflow-hidden">
          <GraphRenderer nodes={data.nodes} edges={data.edges} />
        </div>
      )}
    </div>
  );
}
