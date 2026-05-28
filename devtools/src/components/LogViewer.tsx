import type { Signal } from "@preact/signals";
import { useState, useCallback, useRef } from "preact/hooks";
import type { ObsPanelTab } from "../p2p/types.ts";
import type { RequestResponseData, EventData } from "../obsdc/protocol.ts";
import type { ObsDcConnectionState } from "../obsdc/client.ts";
import { ObsDcScenePanel } from "./obsdc/ObsDcScenePanel.tsx";
import { ObsDcSourcePanel } from "./obsdc/ObsDcSourcePanel.tsx";
import { ObsDcStreamRecordPanel } from "./obsdc/ObsDcStreamRecordPanel.tsx";

interface LogViewerProps {
  obsdcConnectionState: Signal<ObsDcConnectionState>;
  events: Signal<readonly EventData[]>;
  onSendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

const TABS: ReadonlyArray<{ id: ObsPanelTab; label: string }> = [
  { id: "obs-scenes", label: "Scenes" },
  { id: "obs-sources", label: "Sources" },
  { id: "obs-stream-record", label: "Stream / Record" },
];

const MIN_HEIGHT = 120;
const DEFAULT_HEIGHT = 480;

export function LogViewer({ obsdcConnectionState, events, onSendRequest }: LogViewerProps) {
  const [height, setHeight] = useState(DEFAULT_HEIGHT);
  const [activeTab, setActiveTab] = useState<ObsPanelTab>("obs-scenes");
  const draggingRef = useRef(false);
  const startYRef = useRef(0);
  const startHeightRef = useRef(0);

  const handlePointerDown = useCallback(
    (event: PointerEvent) => {
      draggingRef.current = true;
      startYRef.current = event.clientY;
      startHeightRef.current = height;
      (event.target as HTMLElement).setPointerCapture(event.pointerId);
    },
    [height],
  );

  const handlePointerMove = useCallback((event: PointerEvent) => {
    if (!draggingRef.current) {
      return;
    }
    const delta = startYRef.current - event.clientY;
    const maxHeight = Math.floor(window.innerHeight * 0.8);
    const newHeight = Math.min(maxHeight, Math.max(MIN_HEIGHT, startHeightRef.current + delta));
    setHeight(newHeight);
  }, []);

  const handlePointerUp = useCallback(() => {
    draggingRef.current = false;
  }, []);

  return (
    <div class="flex flex-col gap-2">
      <div
        class="flex cursor-row-resize items-center justify-center py-1"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
      >
        <div class="h-1 w-12 rounded-full bg-surface-300" />
      </div>
      <div class="flex items-center gap-1">
        {TABS.map((tab) => {
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => {
                setActiveTab(tab.id);
              }}
              class={`rounded px-4 py-1.5 text-base font-medium ${
                isActive
                  ? "bg-accent-50 text-accent-700 ring-1 ring-inset ring-accent-200"
                  : "text-slate-500 hover:bg-surface-100 hover:text-slate-800"
              }`}
            >
              {tab.label}
            </button>
          );
        })}
      </div>
      <div
        class="flex flex-col rounded-md border border-surface-200 bg-white shadow-sm"
        style={{ height: `${height}px` }}
      >
        <div class="flex-1 overflow-y-auto p-4">
          {activeTab === "obs-scenes" && (
            <ObsDcScenePanel
              connectionState={obsdcConnectionState}
              events={events}
              onSendRequest={onSendRequest}
            />
          )}
          {activeTab === "obs-sources" && (
            <ObsDcSourcePanel
              connectionState={obsdcConnectionState}
              events={events}
              onSendRequest={onSendRequest}
            />
          )}
          {activeTab === "obs-stream-record" && (
            <ObsDcStreamRecordPanel
              connectionState={obsdcConnectionState}
              events={events}
              onSendRequest={onSendRequest}
            />
          )}
        </div>
      </div>
    </div>
  );
}
