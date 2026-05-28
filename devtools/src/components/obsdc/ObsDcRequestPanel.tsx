import type { Signal } from "@preact/signals";
import { useState } from "preact/hooks";
import type { ObsDcConnectionState } from "../../obsdc/client.ts";
import type { RequestResponseData } from "../../obsdc/protocol.ts";

interface ObsDcRequestPanelProps {
  connectionState: Signal<ObsDcConnectionState>;
  onSendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

// よく使うリクエスト
const REQUEST_PRESETS = [
  { label: "GetVersion", requestType: "GetVersion" },
  { label: "GetStats", requestType: "GetStats" },
  { label: "GetSceneList", requestType: "GetSceneList" },
  { label: "GetCurrentProgramScene", requestType: "GetCurrentProgramScene" },
  { label: "GetInputList", requestType: "GetInputList" },
  { label: "GetStreamStatus", requestType: "GetStreamStatus" },
  { label: "GetRecordStatus", requestType: "GetRecordStatus" },
  { label: "GetVirtualCamStatus", requestType: "GetVirtualCamStatus" },
  { label: "GetSceneItemList (current)", requestType: "GetSceneItemList" },
  { label: "GetStudioModeEnabled", requestType: "GetStudioModeEnabled" },
] as const;

export function ObsDcRequestPanel({ connectionState, onSendRequest }: ObsDcRequestPanelProps) {
  const [requestType, setRequestType] = useState("");
  const [requestDataText, setRequestDataText] = useState("");
  const [dataError, setDataError] = useState("");
  const [lastResponse, setLastResponse] = useState<RequestResponseData | null>(null);
  const [sending, setSending] = useState(false);

  const isConnected = connectionState.value === "connected";
  const canSend = isConnected && requestType !== "" && !sending;

  function handlePresetChange(event: Event): void {
    const { value } = event.target as HTMLSelectElement;
    if (value === "") {
      return;
    }
    setRequestType(value);
    setRequestDataText("");
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
    setSending(true);
    setLastResponse(null);

    try {
      const response = await onSendRequest(requestType, requestData);
      setLastResponse(response);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      setDataError(errorMessage);
    } finally {
      setSending(false);
    }
  }

  return (
    <div class="flex flex-col gap-3">
      <div class="text-base font-medium text-slate-800">Request</div>
      <div class="flex items-center gap-2">
        <label class="w-16 shrink-0 text-sm text-slate-600">Preset</label>
        <select
          onChange={handlePresetChange}
          class="flex-1 rounded border border-surface-200 bg-white px-2 py-1 text-sm text-slate-900"
        >
          <option value="">---</option>
          {REQUEST_PRESETS.map((preset) => (
            <option key={preset.requestType} value={preset.requestType}>
              {preset.label}
            </option>
          ))}
        </select>
      </div>
      <div class="flex items-center gap-2">
        <label class="w-16 shrink-0 text-sm text-slate-600">Type</label>
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
        <label class="w-16 shrink-0 pt-1 text-sm text-slate-600">Data</label>
        <div class="flex flex-1 flex-col gap-1">
          <textarea
            value={requestDataText}
            onInput={(event) => {
              setRequestDataText((event.target as HTMLTextAreaElement).value);
              setDataError("");
            }}
            placeholder="{}"
            rows={4}
            class="w-full rounded border border-surface-200 bg-white px-2 py-1 font-mono text-sm text-slate-900 placeholder:text-slate-400"
          />
          {dataError !== "" && <div class="text-xs text-red-700">{dataError}</div>}
        </div>
      </div>
      <div class="flex gap-2">
        <button
          type="button"
          onClick={handleSend}
          disabled={!canSend}
          class="w-20 rounded bg-surface-200 px-3 py-1 text-sm text-slate-900 hover:bg-surface-300 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {sending ? "..." : "Send"}
        </button>
        <button
          type="button"
          onClick={() => {
            setRequestType("");
            setRequestDataText("");
            setDataError("");
            setLastResponse(null);
          }}
          class="w-20 rounded bg-red-100 px-3 py-1 text-sm text-red-800 hover:bg-red-200 hover:text-red-900"
        >
          Clear
        </button>
      </div>
      {lastResponse !== null && (
        <div class="rounded border border-surface-200 bg-white p-3">
          <div class="mb-1 text-sm font-medium text-slate-600">
            {lastResponse.requestStatus.result ? (
              <span class="text-emerald-600">Success</span>
            ) : (
              <span class="text-red-700">
                Failed (code={lastResponse.requestStatus.code}
                {lastResponse.requestStatus.comment
                  ? `, ${lastResponse.requestStatus.comment}`
                  : ""}
                )
              </span>
            )}
          </div>
          {lastResponse.responseData !== undefined && (
            <pre class="max-h-64 overflow-y-auto whitespace-pre-wrap font-mono text-xs text-slate-800">
              {JSON.stringify(lastResponse.responseData, null, 2)}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
