import type { Signal } from "@preact/signals";
import { useRef, useEffect, useState } from "preact/hooks";
import type { ObsDcLogEntry } from "../../obsdc/client.ts";

interface ObsDcEventLogProps {
  logs: Signal<readonly ObsDcLogEntry[]>;
}

const LEVEL_BADGE_COLORS = {
  info: "text-slate-600",
  warn: "text-amber-700",
  error: "text-red-700",
} as const;

const LEVEL_LABELS = {
  info: "INFO",
  warn: "WARN",
  error: "ERROR",
} as const;

function logEntryKey(entry: ObsDcLogEntry): string {
  return `${String(entry.timestamp)}-${entry.level}-${entry.message}`;
}

function formatTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  const millis = String(date.getMilliseconds()).padStart(3, "0");
  return `${hours}:${minutes}:${seconds}.${millis}`;
}

export function ObsDcEventLog({ logs }: ObsDcEventLogProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [filterText, setFilterText] = useState("");

  const entries = logs.value;
  const filteredEntries =
    filterText === ""
      ? entries
      : entries.filter((entry) => entry.message.toLowerCase().includes(filterText.toLowerCase()));

  useEffect(() => {
    if (autoScroll && containerRef.current !== null) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [filteredEntries.length, autoScroll]);

  return (
    <div class="flex flex-1 flex-col overflow-hidden">
      <div class="flex items-center gap-2 border-b border-surface-200 px-3 py-2">
        <span class="text-sm font-medium text-slate-800">Log</span>
        <input
          type="text"
          value={filterText}
          onInput={(event) => {
            setFilterText((event.target as HTMLInputElement).value);
          }}
          placeholder="Filter"
          class="ml-2 flex-1 rounded border border-surface-200 bg-white px-2 py-0.5 text-sm text-slate-900 placeholder:text-slate-400"
        />
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
            setFilterText("");
          }}
          class="rounded bg-red-100 px-2 py-0.5 text-sm text-red-800 hover:bg-red-200 hover:text-red-900"
        >
          Clear
        </button>
      </div>
      <div ref={containerRef} class="flex-1 overflow-y-auto p-3 font-mono text-xs">
        {filteredEntries.length === 0 ? (
          <div class="text-slate-500">No logs</div>
        ) : (
          filteredEntries.map((entry) => (
            <div
              key={logEntryKey(entry)}
              class="border-b border-surface-200/50 py-0.5 leading-relaxed break-all"
            >
              <span class="text-slate-600">{formatTimestamp(entry.timestamp)}</span>{" "}
              <span class={LEVEL_BADGE_COLORS[entry.level]}>[{LEVEL_LABELS[entry.level]}]</span>{" "}
              <span class="text-slate-800">{entry.message}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
