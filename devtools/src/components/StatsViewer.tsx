import type { Signal } from "@preact/signals";
import { useState, useEffect, useRef } from "preact/hooks";

interface StatsViewerProps {
  receivers: Signal<readonly RTCRtpReceiver[]>;
}

// RTCStatsReport の各エントリを表示するテーブル
function StatsEntryTable({
  entries,
  filterText,
}: {
  entries: ReadonlyArray<[string, Record<string, unknown>]>;
  filterText: string;
}) {
  const [collapsedTypes, setCollapsedTypes] = useState<ReadonlySet<string>>(new Set());

  // type ごとにグループ化
  const grouped = new Map<string, ReadonlyArray<[string, Record<string, unknown>]>>();
  for (const entry of entries) {
    const rawType = entry[1].type;
    const type = typeof rawType === "string" ? rawType : "unknown";
    const existing = grouped.get(type);
    if (existing !== undefined) {
      grouped.set(type, [...existing, entry]);
    } else {
      grouped.set(type, [entry]);
    }
  }

  // フィルター適用
  const filteredGrouped = new Map<string, ReadonlyArray<[string, Record<string, unknown>]>>();
  for (const [type, groupEntries] of grouped) {
    if (filterText === "") {
      filteredGrouped.set(type, groupEntries);
      continue;
    }
    const lower = filterText.toLowerCase();
    // type 名がフィルターに一致する場合、グループ全体を表示
    if (type.toLowerCase().includes(lower)) {
      filteredGrouped.set(type, groupEntries);
      continue;
    }
    // 各エントリの値でフィルター
    const filtered = groupEntries.filter(([id, values]) => {
      if (id.toLowerCase().includes(lower)) {
        return true;
      }
      return Object.entries(values).some(
        ([key, value]) =>
          key.toLowerCase().includes(lower) || formatValue(value).toLowerCase().includes(lower),
      );
    });
    if (filtered.length > 0) {
      filteredGrouped.set(type, filtered);
    }
  }

  function toggleType(type: string): void {
    setCollapsedTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) {
        next.delete(type);
      } else {
        next.add(type);
      }
      return next;
    });
  }

  if (filteredGrouped.size === 0) {
    return <div class="text-slate-500">No matching stats</div>;
  }

  return (
    <div class="flex flex-col gap-2">
      {[...filteredGrouped.entries()].map(([type, groupEntries]) => {
        const isCollapsed = collapsedTypes.has(type);
        return (
          <div key={type} class="rounded border border-surface-200">
            <button
              type="button"
              onClick={() => {
                toggleType(type);
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
              {type}
              <span class="text-slate-600">({groupEntries.length})</span>
            </button>
            {!isCollapsed && (
              <div class="border-t border-surface-200">
                {groupEntries.map(([id, values]) => (
                  <div key={id} class="border-b border-surface-200/50 px-3 py-1.5 last:border-b-0">
                    <div class="mb-1 text-xs font-medium text-slate-800">{id}</div>
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
                ))}
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

function receiverKey(receiver: RTCRtpReceiver): string {
  return receiver.track.id;
}

function receiverLabel(receiver: RTCRtpReceiver): string {
  const { id, kind } = receiver.track;
  const idPrefix = id.slice(0, 8);
  return `${kind}:${idPrefix}`;
}

export function StatsViewer({ receivers }: StatsViewerProps) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [statsMap, setStatsMap] = useState<
    ReadonlyMap<number, ReadonlyArray<[string, Record<string, unknown>]>>
  >(new Map());
  const [filterText, setFilterText] = useState("");
  const receiversRef = useRef(receivers);
  receiversRef.current = receivers;

  useEffect(() => {
    async function poll(): Promise<void> {
      const currentReceivers = receiversRef.current.value;
      const newMap = new Map<number, ReadonlyArray<[string, Record<string, unknown>]>>();

      for (let i = 0; i < currentReceivers.length; i++) {
        try {
          const stats = await currentReceivers[i].getStats();
          const entries: Array<[string, Record<string, unknown>]> = [];
          for (const [key, value] of stats) {
            entries.push([key, value as Record<string, unknown>]);
          }
          newMap.set(i, entries);
        } catch {
          // getStats() が失敗した場合は空にする
        }
      }

      setStatsMap(newMap);
    }

    void poll();
    const timerId = setInterval(() => void poll(), 1000);

    return () => {
      clearInterval(timerId);
    };
  }, []);

  const receiverList = receivers.value;

  if (receiverList.length === 0) {
    return <div class="flex h-full items-center justify-center text-slate-500">No tracks</div>;
  }

  // 選択インデックスが範囲外の場合に補正
  const safeIndex = selectedIndex < receiverList.length ? selectedIndex : 0;
  const currentEntries = statsMap.get(safeIndex) ?? [];

  return (
    <div class="flex h-full flex-col">
      <div class="flex items-center gap-1 border-b border-surface-200 px-3 py-2">
        {receiverList.map((receiver, index) => {
          const isActive = index === safeIndex;
          return (
            <button
              key={receiverKey(receiver)}
              type="button"
              onClick={() => {
                setSelectedIndex(index);
              }}
              class={`rounded px-3 py-1 text-sm ${
                isActive
                  ? "bg-accent-50 text-accent-700 ring-1 ring-inset ring-accent-200"
                  : "text-slate-500 hover:bg-surface-100 hover:text-slate-800"
              }`}
            >
              {receiverLabel(receiver)}
            </button>
          );
        })}
        <input
          type="text"
          value={filterText}
          onInput={(event) => {
            setFilterText((event.target as HTMLInputElement).value);
          }}
          placeholder="Filter"
          class="ml-auto rounded border border-surface-200 bg-white px-2 py-0.5 text-sm text-slate-900 placeholder:text-slate-400"
        />
      </div>
      <div class="flex-1 overflow-y-scroll p-3 font-mono text-sm">
        <StatsEntryTable entries={currentEntries} filterText={filterText} />
      </div>
    </div>
  );
}
