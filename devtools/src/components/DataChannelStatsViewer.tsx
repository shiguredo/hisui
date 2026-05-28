import { useState, useEffect, useRef } from "preact/hooks";

interface DataChannelStatsViewerProps {
  getPeerConnectionStats: () => Promise<RTCStatsReport | null>;
}

// 統計エントリを表示するテーブル
function StatsEntryTable({
  entries,
  filterText,
}: {
  entries: ReadonlyArray<[string, Record<string, unknown>]>;
  filterText: string;
}) {
  const [collapsedIds, setCollapsedIds] = useState<ReadonlySet<string>>(new Set());

  // フィルター適用
  const filtered =
    filterText === ""
      ? entries
      : entries.filter(([id, values]) => {
          const lower = filterText.toLowerCase();
          if (id.toLowerCase().includes(lower)) {
            return true;
          }
          const rawType = values.type;
          const type = typeof rawType === "string" ? rawType : "";
          if (type.toLowerCase().includes(lower)) {
            return true;
          }
          return Object.entries(values).some(
            ([key, value]) =>
              key.toLowerCase().includes(lower) || formatValue(value).toLowerCase().includes(lower),
          );
        });

  function toggleId(id: string): void {
    setCollapsedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  if (filtered.length === 0) {
    return <div class="text-slate-500">No matching stats</div>;
  }

  return (
    <div class="flex flex-col gap-2">
      {filtered.map(([id, values]) => {
        const isCollapsed = collapsedIds.has(id);
        const rawType = values.type;
        const type = typeof rawType === "string" ? rawType : "unknown";
        return (
          <div key={id} class="rounded border border-surface-200">
            <button
              type="button"
              onClick={() => {
                toggleId(id);
              }}
              class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm font-medium text-slate-800 hover:bg-surface-100"
            >
              <svg width="14" height="14" viewBox="0 0 20 20" class="shrink-0 text-slate-600">
                {isCollapsed ? (
                  <path
                    d="M8 5l5 5-5 5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                ) : (
                  <path
                    d="M5 8l5 5 5-5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                )}
              </svg>
              <span class="text-slate-800">[{type}]</span>
              {id}
            </button>
            {!isCollapsed && (
              <div class="border-t border-surface-200 px-3 py-1.5">
                <table class="w-full text-sm">
                  <tbody>
                    {Object.entries(values)
                      .filter(([key]) => key !== "type" && key !== "id")
                      .map(([key, value]) => (
                        <tr key={key} class="border-b border-surface-200/30 last:border-b-0">
                          <td class="w-60 py-0.5 pr-3 text-slate-800">{key}</td>
                          <td class="py-0.5 text-slate-800">{formatValue(value)}</td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) {
    return "null";
  }
  if (typeof value === "object") {
    return JSON.stringify(value);
  }
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return JSON.stringify(value);
}

export function DataChannelStatsViewer({ getPeerConnectionStats }: DataChannelStatsViewerProps) {
  const [entries, setEntries] = useState<ReadonlyArray<[string, Record<string, unknown>]>>([]);
  const [filterText, setFilterText] = useState("");
  const getPeerConnectionStatsRef = useRef(getPeerConnectionStats);
  getPeerConnectionStatsRef.current = getPeerConnectionStats;

  useEffect(() => {
    async function poll(): Promise<void> {
      const stats = await getPeerConnectionStatsRef.current();
      if (stats === null) {
        setEntries([]);
        return;
      }

      const result: Array<[string, Record<string, unknown>]> = [];
      for (const [key, value] of stats) {
        const record = value as Record<string, unknown>;
        const rawType = record.type;
        const type = typeof rawType === "string" ? rawType : "";
        // data-channel と transport の統計を抽出
        if (type === "data-channel" || type === "transport") {
          result.push([key, record]);
        }
      }
      setEntries(result);
    }

    void poll();
    const timerId = setInterval(() => void poll(), 1000);

    return () => {
      clearInterval(timerId);
    };
  }, []);

  if (entries.length === 0) {
    return <div class="flex h-full items-center justify-center text-slate-500">No stats</div>;
  }

  return (
    <div class="flex h-full flex-col">
      <div class="flex items-center gap-2 border-b border-surface-200 px-3 py-2">
        <input
          type="text"
          value={filterText}
          onInput={(event) => {
            setFilterText((event.target as HTMLInputElement).value);
          }}
          placeholder="Filter"
          class="flex-1 rounded border border-surface-200 bg-white px-2 py-0.5 text-sm text-slate-900 placeholder:text-slate-400"
        />
      </div>
      <div class="flex-1 overflow-y-scroll p-3 font-mono text-sm">
        <StatsEntryTable entries={entries} filterText={filterText} />
      </div>
    </div>
  );
}
