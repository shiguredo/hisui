import type { Signal } from "@preact/signals";
import { useRef } from "preact/hooks";
import type { ObsDcConnectionState, ObsDcConnectionConfig } from "../../obsdc/client.ts";

interface ObsDcConnectionPanelProps {
  connectionState: Signal<ObsDcConnectionState>;
  lastError: Signal<string | null>;
  onConnect: (config: ObsDcConnectionConfig) => void;
  onDisconnect: () => void;
}

const DEFAULT_URL = "ws://localhost:4455";

const STATE_LABELS: Record<ObsDcConnectionState, string> = {
  disconnected: "Disconnected",
  connecting: "Connecting...",
  authenticating: "Authenticating...",
  connected: "Connected",
};

const STATE_COLORS: Record<ObsDcConnectionState, string> = {
  disconnected: "text-slate-800",
  connecting: "text-amber-700",
  authenticating: "text-amber-700",
  connected: "text-emerald-600",
};

export function ObsDcConnectionPanel({
  connectionState,
  lastError,
  onConnect,
  onDisconnect,
}: ObsDcConnectionPanelProps) {
  const urlInputRef = useRef<HTMLInputElement>(null);
  const passwordInputRef = useRef<HTMLInputElement>(null);
  const state = connectionState.value;
  const isActive = state === "connecting" || state === "authenticating" || state === "connected";
  const isDisabled = state === "connecting" || state === "authenticating";

  function handleClick(): void {
    if (isActive) {
      onDisconnect();
    } else {
      const url = urlInputRef.current?.value.trim() ?? DEFAULT_URL;
      const password = passwordInputRef.current?.value ?? "";
      onConnect({ url, password });
    }
  }

  const buttonLabel = isActive ? "Disconnect" : "Connect";

  return (
    <div class="flex flex-col gap-3">
      <div class="flex items-center gap-2">
        <span class="text-sm text-slate-800">Status:</span>
        <span class={`text-sm font-medium ${STATE_COLORS[state]}`}>{STATE_LABELS[state]}</span>
      </div>
      {lastError.value !== null && (
        <div class="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800">
          {lastError.value}
        </div>
      )}
      <label class="text-base font-medium text-slate-800">WebSocket URL</label>
      <input
        ref={urlInputRef}
        type="text"
        defaultValue={DEFAULT_URL}
        disabled={isActive}
        class="field-control px-3 py-2 text-base"
      />
      <label class="text-base font-medium text-slate-800">Password</label>
      <input
        ref={passwordInputRef}
        type="password"
        placeholder="(optional)"
        disabled={isActive}
        class="field-control px-3 py-2 text-base"
      />
      <button
        type="button"
        onClick={handleClick}
        disabled={isDisabled}
        class="w-full rounded-md bg-accent-600 px-4 py-2 text-base font-medium text-white shadow-sm hover:bg-accent-500 disabled:opacity-50"
      >
        {buttonLabel}
      </button>
    </div>
  );
}
