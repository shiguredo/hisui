import type { Signal } from "@preact/signals";
import type { ConnectionState, CloseMessage } from "../p2p/types.ts";

interface ConnectionStatusProps {
  connectionState: Signal<ConnectionState>;
  closeMessage: Signal<CloseMessage | null>;
  lastError: Signal<Error | null>;
}

const STATE_LABELS: Record<ConnectionState, string> = {
  idle: "Idle",
  bootstrapping: "Bootstrapping",
  connecting: "Connecting",
  connected: "Connected",
  disconnecting: "Disconnecting",
  closed: "Closed",
};

const STATE_COLORS: Record<ConnectionState, string> = {
  idle: "bg-surface-500",
  bootstrapping: "bg-amber-400",
  connecting: "bg-amber-400",
  connected: "bg-emerald-500",
  disconnecting: "bg-amber-400",
  closed: "bg-red-500",
};

export function ConnectionStatus({
  connectionState,
  closeMessage,
  lastError,
}: ConnectionStatusProps) {
  const state = connectionState.value;
  const close = closeMessage.value;
  const error = lastError.value;

  return (
    <div class="flex flex-col gap-2">
      <h3 class="text-base font-medium text-slate-600">Connection Status</h3>
      <div class="flex items-center gap-2">
        <span class={`inline-block h-3 w-3 rounded-full ${STATE_COLORS[state]}`} />
        <span class="text-base text-slate-900">{STATE_LABELS[state]}</span>
      </div>
      {close !== null && (
        <div class="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800">
          <div>Code: {close.code}</div>
          <div>Reason: {close.reason}</div>
        </div>
      )}
      {error !== null && (
        <div class="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800">
          {error.message}
        </div>
      )}
    </div>
  );
}
