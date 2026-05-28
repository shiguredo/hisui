import type { Signal } from "@preact/signals";
import { useState, useEffect, useCallback } from "preact/hooks";
import type { ObsDcConnectionState } from "../../obsdc/client.ts";
import type { RequestResponseData, EventData } from "../../obsdc/protocol.ts";

interface ObsDcStreamRecordPanelProps {
  connectionState: Signal<ObsDcConnectionState>;
  events: Signal<readonly EventData[]>;
  onSendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

interface StreamStatus {
  outputActive: boolean;
  outputReconnecting: boolean;
  outputTimecode: string;
  outputBytes: number;
}

interface RecordStatus {
  outputActive: boolean;
  outputPaused: boolean;
  outputTimecode: string;
  outputBytes: number;
}

interface VirtualCamStatus {
  outputActive: boolean;
}

interface RecordStatusDisplay {
  label: string;
  className: string;
}

function getRecordStatusDisplay(status: RecordStatus): RecordStatusDisplay {
  if (!status.outputActive) {
    return { label: "OFF", className: "text-slate-600" };
  }
  if (status.outputPaused) {
    return { label: "PAUSED", className: "text-amber-700" };
  }
  return { label: "REC", className: "text-red-700" };
}

interface StreamSectionProps {
  streamStatus: StreamStatus | null;
  onSendRequest: ObsDcStreamRecordPanelProps["onSendRequest"];
}

function StreamSection({ streamStatus, onSendRequest }: StreamSectionProps) {
  async function handleStartStream(): Promise<void> {
    try {
      await onSendRequest("StartStream");
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  async function handleStopStream(): Promise<void> {
    try {
      await onSendRequest("StopStream");
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  return (
    <div class="flex flex-col gap-2 rounded border border-surface-200 p-3">
      <div class="flex items-center gap-2 text-sm">
        <span class="font-medium text-slate-800">Stream</span>
        {streamStatus !== null && (
          <span
            class={`text-xs ${streamStatus.outputActive ? "text-emerald-600" : "text-slate-600"}`}
          >
            {streamStatus.outputActive ? "LIVE" : "OFF"}
          </span>
        )}
        {streamStatus?.outputActive && (
          <span class="text-xs text-slate-800">{streamStatus.outputTimecode}</span>
        )}
      </div>
      <div class="flex gap-2">
        <button
          type="button"
          onClick={() => {
            void handleStartStream();
          }}
          disabled={streamStatus?.outputActive === true}
          class="rounded bg-green-800/60 px-3 py-1 text-sm text-green-200 hover:bg-green-700/60 disabled:opacity-50"
        >
          Start
        </button>
        <button
          type="button"
          onClick={() => {
            void handleStopStream();
          }}
          disabled={streamStatus?.outputActive !== true}
          class="rounded bg-red-100 px-3 py-1 text-sm text-red-800 hover:bg-red-800/60 disabled:opacity-50"
        >
          Stop
        </button>
      </div>
    </div>
  );
}

interface RecordSectionProps {
  recordStatus: RecordStatus | null;
  onSendRequest: ObsDcStreamRecordPanelProps["onSendRequest"];
}

function RecordSection({ recordStatus, onSendRequest }: RecordSectionProps) {
  const statusDisplay = recordStatus === null ? null : getRecordStatusDisplay(recordStatus);

  async function handleStartRecord(): Promise<void> {
    try {
      await onSendRequest("StartRecord");
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  async function handleStopRecord(): Promise<void> {
    try {
      await onSendRequest("StopRecord");
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  async function handleToggleRecordPause(): Promise<void> {
    try {
      await onSendRequest("ToggleRecordPause");
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  return (
    <div class="flex flex-col gap-2 rounded border border-surface-200 p-3">
      <div class="flex items-center gap-2 text-sm">
        <span class="font-medium text-slate-800">Record</span>
        {statusDisplay !== null && (
          <span class={`text-xs ${statusDisplay.className}`}>{statusDisplay.label}</span>
        )}
        {recordStatus?.outputActive && (
          <span class="text-xs text-slate-800">{recordStatus.outputTimecode}</span>
        )}
      </div>
      <div class="flex gap-2">
        <button
          type="button"
          onClick={() => {
            void handleStartRecord();
          }}
          disabled={recordStatus?.outputActive === true}
          class="rounded bg-red-800/60 px-3 py-1 text-sm text-red-800 hover:bg-red-700/60 disabled:opacity-50"
        >
          Start
        </button>
        <button
          type="button"
          onClick={() => {
            void handleStopRecord();
          }}
          disabled={recordStatus?.outputActive !== true}
          class="rounded bg-surface-200 px-3 py-1 text-sm text-slate-800 hover:bg-surface-300 disabled:opacity-50"
        >
          Stop
        </button>
        <button
          type="button"
          onClick={() => {
            void handleToggleRecordPause();
          }}
          disabled={recordStatus?.outputActive !== true}
          class="rounded bg-yellow-800/60 px-3 py-1 text-sm text-yellow-200 hover:bg-yellow-700/60 disabled:opacity-50"
        >
          {recordStatus?.outputPaused ? "Resume" : "Pause"}
        </button>
      </div>
    </div>
  );
}

interface VirtualCamSectionProps {
  virtualCamStatus: VirtualCamStatus | null;
  onSendRequest: ObsDcStreamRecordPanelProps["onSendRequest"];
}

function VirtualCamSection({ virtualCamStatus, onSendRequest }: VirtualCamSectionProps) {
  async function handleToggleVirtualCam(): Promise<void> {
    try {
      await onSendRequest("ToggleVirtualCam");
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  return (
    <div class="flex flex-col gap-2 rounded border border-surface-200 p-3">
      <div class="flex items-center gap-2 text-sm">
        <span class="font-medium text-slate-800">Virtual Camera</span>
        {virtualCamStatus !== null && (
          <span
            class={`text-xs ${virtualCamStatus.outputActive ? "text-emerald-600" : "text-slate-600"}`}
          >
            {virtualCamStatus.outputActive ? "ON" : "OFF"}
          </span>
        )}
      </div>
      <div class="flex gap-2">
        <button
          type="button"
          onClick={() => {
            void handleToggleVirtualCam();
          }}
          class="rounded bg-surface-200 px-3 py-1 text-sm text-slate-800 hover:bg-surface-300 disabled:opacity-50"
        >
          Toggle
        </button>
      </div>
    </div>
  );
}

const REFRESH_EVENT_TYPES = [
  "StreamStateChanged",
  "RecordStateChanged",
  "VirtualcamStateChanged",
] as const;

export function ObsDcStreamRecordPanel({
  connectionState,
  events,
  onSendRequest,
}: ObsDcStreamRecordPanelProps) {
  const [streamStatus, setStreamStatus] = useState<StreamStatus | null>(null);
  const [recordStatus, setRecordStatus] = useState<RecordStatus | null>(null);
  const [virtualCamStatus, setVirtualCamStatus] = useState<VirtualCamStatus | null>(null);
  const [loading, setLoading] = useState(false);

  const isConnected = connectionState.value === "connected";

  const fetchStatus = useCallback(async (): Promise<void> => {
    if (!isConnected) {
      return;
    }
    setLoading(true);
    try {
      const [streamResponse, recordResponse, virtualCamResponse] = await Promise.all([
        onSendRequest("GetStreamStatus"),
        onSendRequest("GetRecordStatus"),
        onSendRequest("GetVirtualCamStatus"),
      ]);
      if (streamResponse.requestStatus.result && streamResponse.responseData) {
        setStreamStatus(streamResponse.responseData as unknown as StreamStatus);
      }
      if (recordResponse.requestStatus.result && recordResponse.responseData) {
        setRecordStatus(recordResponse.responseData as unknown as RecordStatus);
      }
      if (virtualCamResponse.requestStatus.result && virtualCamResponse.responseData) {
        setVirtualCamStatus(virtualCamResponse.responseData as unknown as VirtualCamStatus);
      }
    } catch {
      // 失敗時は前回の状態を維持する
    } finally {
      setLoading(false);
    }
  }, [isConnected, onSendRequest]);

  useEffect(() => {
    if (isConnected) {
      void fetchStatus();
      return;
    }
    setStreamStatus(null);
    setRecordStatus(null);
    setVirtualCamStatus(null);
  }, [isConnected, fetchStatus]);

  // イベントで状態変化を検知してリフレッシュする
  const eventCount = events.value.length;
  useEffect(() => {
    if (eventCount === 0) {
      return;
    }
    const latestEvent = events.value.at(-1);
    if (latestEvent === undefined) {
      return;
    }
    if (
      REFRESH_EVENT_TYPES.includes(latestEvent.eventType as (typeof REFRESH_EVENT_TYPES)[number])
    ) {
      void fetchStatus();
    }
  }, [eventCount, events.value, fetchStatus]);

  return (
    <div class="flex flex-col gap-4">
      <div class="flex items-center gap-2">
        <span class="text-base font-medium text-slate-800">Stream / Record</span>
        <button
          type="button"
          onClick={() => {
            void fetchStatus();
          }}
          disabled={!isConnected || loading}
          class="ml-auto rounded bg-surface-200 px-2 py-0.5 text-xs text-slate-600 hover:bg-surface-300 disabled:opacity-50"
        >
          Refresh
        </button>
      </div>

      {!isConnected ? (
        <div class="text-sm text-slate-600">Not connected</div>
      ) : (
        <div class="flex flex-col gap-4">
          <StreamSection streamStatus={streamStatus} onSendRequest={onSendRequest} />
          <RecordSection recordStatus={recordStatus} onSendRequest={onSendRequest} />
          <VirtualCamSection virtualCamStatus={virtualCamStatus} onSendRequest={onSendRequest} />
        </div>
      )}
    </div>
  );
}
