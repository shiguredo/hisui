import type { Signal } from "@preact/signals";
import type { DataChannelState } from "../p2p/types.ts";

interface DataChannelStatusProps {
  dataChannelState: Signal<DataChannelState>;
}

function stateColor(state: RTCDataChannelState | "not-created"): string {
  switch (state) {
    case "open": {
      return "text-emerald-600";
    }
    case "connecting": {
      return "text-amber-600";
    }
    case "closing": {
      return "text-amber-600";
    }
    case "closed": {
      return "text-red-700";
    }
    case "not-created": {
      return "text-slate-500";
    }
  }
}

export function DataChannelStatus({ dataChannelState }: DataChannelStatusProps) {
  const state = dataChannelState.value;

  return (
    <div class="flex flex-col gap-2">
      <h3 class="text-base font-medium text-slate-600">DataChannel</h3>
      <div class="flex flex-col gap-1 text-base">
        <div class="flex justify-between">
          <span class="text-slate-800">signaling</span>
          <span class={stateColor(state.signaling)}>{state.signaling}</span>
        </div>
        <div class="flex justify-between">
          <span class="text-slate-800">obsdc</span>
          <span class={stateColor(state.obsdc)}>{state.obsdc}</span>
        </div>
      </div>
    </div>
  );
}
