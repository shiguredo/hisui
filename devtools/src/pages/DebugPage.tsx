import type { JSX } from "preact";
import type { Signal } from "@preact/signals";
import { useRef, useEffect, useState, useMemo } from "preact/hooks";
import type { LogEntry, DebugTab, DataChannelState } from "../p2p/types.ts";
import type { RequestResponseData } from "../obsdc/protocol.ts";
import requestPresets from "../obsdc.json";
import { useP2PClient } from "../context/P2PClientProvider.tsx";
import { StatsViewer } from "../components/StatsViewer.tsx";
import { DataChannelStatsViewer } from "../components/DataChannelStatsViewer.tsx";

interface RequestPreset {
  requestType: string;
  requestData?: Record<string, unknown>;
}

function formatTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  const millis = String(date.getMilliseconds()).padStart(3, "0");
  return `${hours}:${minutes}:${seconds}.${millis}`;
}

const LEVEL_BADGE_COLORS = {
  info: "text-slate-600",
  warn: "text-amber-700",
  error: "text-red-700",
} as const;

const ROW_STYLES = {
  info: "text-slate-800",
  warn: "text-slate-800 bg-amber-50",
  error: "text-slate-800 bg-red-50",
} as const;

const LEVEL_LABELS = {
  info: "INFO",
  warn: "WARN",
  error: "ERROR",
} as const;

function logEntryKey(entry: LogEntry): string {
  return `${String(entry.timestamp)}-${entry.category}-${entry.level}-${entry.message}`;
}

const TABS: ReadonlyArray<{ id: DebugTab; label: string }> = [
  { id: "pc", label: "PeerConnection" },
  { id: "signaling", label: "Signaling" },
  { id: "obsdc", label: "OBS WebSocket" },
  { id: "track-stats", label: "TrackStats" },
  { id: "datachannel-stats", label: "DataChannelStats" },
];

interface LogEntryRowProps {
  entry: LogEntry;
  expanded: boolean;
  onToggle: () => void;
}

function LogEntryRow({ entry, expanded, onToggle }: LogEntryRowProps) {
  const hasMultipleLines = entry.message.includes("\n");
  const firstLine = hasMultipleLines ? entry.message.split("\n")[0] : entry.message;

  const rowClass = `${ROW_STYLES[entry.level]} border-b border-surface-200/50 py-1 leading-relaxed`;

  if (!hasMultipleLines) {
    return (
      <div class={rowClass}>
        <span class="text-slate-800">{formatTimestamp(entry.timestamp)}</span>{" "}
        <span class={LEVEL_BADGE_COLORS[entry.level]}>[{LEVEL_LABELS[entry.level]}]</span>{" "}
        {entry.message}
      </div>
    );
  }

  return (
    <div class={rowClass}>
      <button type="button" onClick={onToggle} class="w-full text-left">
        <span class="text-slate-800">{formatTimestamp(entry.timestamp)}</span>{" "}
        <span class={LEVEL_BADGE_COLORS[entry.level]}>[{LEVEL_LABELS[entry.level]}]</span>{" "}
        <svg
          width="14"
          height="14"
          viewBox="0 0 20 20"
          class="inline-block align-middle text-slate-600"
        >
          {expanded ? (
            <path
              d="M5 8l5 5 5-5"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          ) : (
            <path
              d="M8 5l5 5-5 5"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          )}
        </svg>{" "}
        {firstLine}
      </button>
      {expanded && (
        <pre class="mt-1 ml-4 whitespace-pre-wrap text-slate-600">
          {entry.message.slice(firstLine.length + 1)}
        </pre>
      )}
    </div>
  );
}

interface RpcSendFormProps {
  dataChannelState: Signal<DataChannelState>;
  onSendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

function RpcSendForm({ dataChannelState, onSendRequest }: RpcSendFormProps) {
  const [requestType, setRequestType] = useState("");
  const [requestDataText, setRequestDataText] = useState("");
  const [dataError, setDataError] = useState("");

  const isOpen = dataChannelState.value.obsdc === "open";
  const canSend = isOpen && requestType !== "";

  function handlePresetChange(event: Event): void {
    const requestTypeValue = (event.target as HTMLSelectElement).value;
    if (requestTypeValue === "") {
      return;
    }
    const preset = (requestPresets as RequestPreset[]).find(
      (item) => item.requestType === requestTypeValue,
    );
    if (preset === undefined) {
      return;
    }
    setRequestType(preset.requestType);
    if (preset.requestData !== undefined) {
      setRequestDataText(JSON.stringify(preset.requestData, null, 2));
    } else {
      setRequestDataText("");
    }
    setDataError("");
  }

  async function handleSend(): Promise<void> {
    if (!canSend) {
      return;
    }

    let requestData: Record<string, unknown> | undefined;
    if (requestDataText.trim() !== "") {
      try {
        const parsed: unknown = JSON.parse(requestDataText);
        if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
          setDataError("requestData must be an object");
          return;
        }
        requestData = parsed as Record<string, unknown>;
      } catch {
        setDataError("requestData is not valid JSON");
        return;
      }
    }

    setDataError("");
    try {
      await onSendRequest(requestType, requestData);
    } catch (error: unknown) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      setDataError(errorMessage);
    }
  }

  return (
    <div class="flex flex-col gap-2 border-b border-surface-200 px-3 py-2">
      <div class="flex items-center gap-2">
        <label class="w-20 shrink-0 text-sm text-slate-600">Preset</label>
        <select
          onChange={handlePresetChange}
          class="rounded border border-surface-200 bg-white px-2 py-1 text-sm text-slate-900"
        >
          <option value="">Select</option>
          {(requestPresets as RequestPreset[]).map((preset) => (
            <option key={preset.requestType} value={preset.requestType}>
              {preset.requestType}
            </option>
          ))}
        </select>
      </div>
      <div class="flex items-center gap-2">
        <label class="w-20 shrink-0 text-sm text-slate-600">Type</label>
        <input
          type="text"
          value={requestType}
          onInput={(event) => {
            setRequestType((event.target as HTMLInputElement).value);
          }}
          placeholder="requestType"
          class="flex-1 rounded border border-surface-200 bg-white px-2 py-1 text-sm text-slate-900 placeholder:text-slate-400"
        />
      </div>
      <div class="flex gap-2">
        <label class="w-20 shrink-0 pt-1 text-sm text-slate-600">Data</label>
        <div class="flex flex-1 flex-col gap-1">
          <div class="relative">
            <textarea
              value={requestDataText}
              onInput={(event) => {
                setRequestDataText((event.target as HTMLTextAreaElement).value);
                setDataError("");
              }}
              placeholder="{}"
              rows={8}
              class="w-full overflow-y-scroll rounded border border-surface-200 bg-white px-2 py-1 pr-24 font-mono text-sm text-slate-900 placeholder:text-slate-400"
            />
            <button
              type="button"
              onClick={() => {
                try {
                  const parsed: unknown = JSON.parse(requestDataText);
                  setRequestDataText(JSON.stringify(parsed, null, 2));
                  setDataError("");
                } catch {
                  setDataError("requestData is not valid JSON");
                }
              }}
              disabled={requestDataText.trim() === ""}
              class="absolute top-1.5 right-6 rounded bg-surface-200 px-4 py-1.5 text-base text-slate-600 hover:bg-surface-300 hover:text-slate-900 disabled:cursor-not-allowed disabled:opacity-50"
            >
              Pretty
            </button>
          </div>
          {dataError !== "" && <div class="text-xs text-red-700">{dataError}</div>}
        </div>
      </div>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onClick={handleSend}
          disabled={!canSend}
          class="w-20 rounded bg-surface-200 px-3 py-1 text-sm text-slate-900 hover:bg-surface-300 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Send
        </button>
        {!isOpen && <span class="text-xs text-slate-600">RPC DataChannel is not open</span>}
        <button
          type="button"
          onClick={() => {
            setRequestType("");
            setRequestDataText("");
            setDataError("");
          }}
          class="ml-auto w-20 rounded bg-red-100 px-3 py-1 text-sm text-red-800 hover:bg-red-200 hover:text-red-900"
        >
          Clear
        </button>
      </div>
    </div>
  );
}

export function DebugPage() {
  const client = useP2PClient();
  const { logs } = client.state;
  const { receivers } = client.state;
  const { dataChannelState } = client.state;

  const containerRef = useRef<HTMLDivElement>(null);
  const [activeTab, setActiveTab] = useState<DebugTab>("pc");
  const [autoScroll, setAutoScroll] = useState(true);
  const [expandedSet, setExpandedSet] = useState<ReadonlySet<number>>(new Set());
  const [filterText, setFilterText] = useState("");

  const allEntries = logs.value;
  const categoryEntries = allEntries.filter((entry) => entry.category === activeTab);
  const entries =
    filterText === ""
      ? categoryEntries
      : categoryEntries.filter((entry) =>
          entry.message.toLowerCase().includes(filterText.toLowerCase()),
        );

  useEffect(() => {
    if (autoScroll && containerRef.current !== null) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [entries.length, autoScroll]);

  const [copied, setCopied] = useState(false);

  const logsText = useMemo(
    () =>
      entries
        .map(
          (entry) =>
            `${formatTimestamp(entry.timestamp)} [${LEVEL_LABELS[entry.level]}] ${entry.message}`,
        )
        .join("\n"),
    [entries],
  );

  async function copyLogs(): Promise<void> {
    try {
      await navigator.clipboard.writeText(logsText);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch {
      // クリップボード API が使えない環境では何もしない
    }
  }

  const isStatsTab = activeTab === "track-stats" || activeTab === "datachannel-stats";

  function countByCategory(category: DebugTab): number {
    if (category === "track-stats" || category === "datachannel-stats") {
      return 0;
    }
    return allEntries.filter((entry) => entry.category === category).length;
  }

  function toggleEntry(index: number): void {
    setExpandedSet((prev) => {
      const next = new Set(prev);
      if (next.has(index)) {
        next.delete(index);
      } else {
        next.add(index);
      }
      return next;
    });
  }

  function expandAll(): void {
    const indices = new Set<number>();
    for (let i = 0; i < entries.length; i++) {
      if (entries[i].message.includes("\n")) {
        indices.add(i);
      }
    }
    setExpandedSet(indices);
  }

  function collapseAll(): void {
    setExpandedSet(new Set());
  }

  function renderTabPanel(): JSX.Element {
    if (activeTab === "track-stats") {
      return <StatsViewer receivers={receivers} />;
    }
    if (activeTab === "datachannel-stats") {
      return (
        <DataChannelStatsViewer
          getPeerConnectionStats={async () => client.getPeerConnectionStats()}
        />
      );
    }
    return (
      <>
        {activeTab === "obsdc" && (
          <RpcSendForm
            dataChannelState={dataChannelState}
            onSendRequest={async (requestType, requestData) =>
              client.sendRequest(requestType, requestData)
            }
          />
        )}
        <div class="flex items-center gap-2 border-b border-surface-200 px-3 py-2">
          <button
            type="button"
            onClick={() => {
              if (expandedSet.size > 0) {
                collapseAll();
              } else {
                expandAll();
              }
            }}
            class="rounded p-1 text-slate-600 hover:bg-surface-100 hover:text-slate-800"
          >
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
              {expandedSet.size > 0 ? (
                <path
                  d="M5 8l5 5 5-5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              ) : (
                <path
                  d="M8 5l5 5-5 5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              )}
            </svg>
          </button>
          <input
            type="text"
            value={filterText}
            onInput={(event) => {
              setFilterText((event.target as HTMLInputElement).value);
            }}
            placeholder="Filter"
            class="ml-2 flex-1 rounded border border-surface-200 bg-white px-2 py-0.5 text-sm text-slate-900 placeholder:text-slate-400"
          />
          <button
            type="button"
            onClick={() => {
              void copyLogs();
            }}
            disabled={entries.length === 0}
            class="w-20 rounded bg-surface-200 px-2 py-0.5 text-sm text-slate-600 hover:bg-surface-300 hover:text-slate-900 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {copied ? "Copied!" : "Copy"}
          </button>
        </div>
        <div ref={containerRef} class="flex-1 overflow-y-scroll p-3 font-mono text-sm">
          {entries.length === 0 ? (
            <div class="text-slate-500">No logs</div>
          ) : (
            entries.map((entry, index) => (
              <LogEntryRow
                key={logEntryKey(entry)}
                entry={entry}
                expanded={expandedSet.has(index)}
                onToggle={() => {
                  toggleEntry(index);
                }}
              />
            ))
          )}
        </div>
      </>
    );
  }

  return (
    <div class="flex flex-1 flex-col overflow-hidden p-4">
      <div class="flex items-center gap-1">
        {TABS.map((tab) => {
          const isActive = activeTab === tab.id;
          const count = countByCategory(tab.id);
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => {
                setActiveTab(tab.id);
                setExpandedSet(new Set());
              }}
              class={`rounded px-4 py-1.5 text-base font-medium ${
                isActive
                  ? "bg-accent-50 text-accent-700 ring-1 ring-inset ring-accent-200"
                  : "text-slate-500 hover:bg-surface-100 hover:text-slate-800"
              }`}
            >
              {tab.label}
              {count > 0 && <span class="ml-1 text-sm text-slate-600">({count})</span>}
            </button>
          );
        })}
        {!isStatsTab && (
          <div class="ml-auto flex items-center gap-3">
            <label class="flex items-center gap-1.5 text-sm text-slate-600">
              <input
                type="checkbox"
                checked={autoScroll}
                onChange={(event) => {
                  setAutoScroll((event.target as HTMLInputElement).checked);
                }}
                class="accent-accent-500"
              />
              Auto Scroll
            </label>
            <button
              type="button"
              onClick={() => {
                logs.value = [];
                setExpandedSet(new Set());
                setFilterText("");
              }}
              class="rounded bg-red-100 px-2 py-0.5 text-sm text-red-800 hover:bg-red-200 hover:text-red-900"
            >
              Clear
            </button>
          </div>
        )}
      </div>
      <div class="mt-2 flex flex-1 flex-col overflow-hidden rounded-md border border-surface-200 bg-white shadow-sm">
        {renderTabPanel()}
      </div>
    </div>
  );
}
