import type { Signal } from "@preact/signals";
import type { HelloData } from "../../obsdc/protocol.ts";

interface ObsDcServerInfoProps {
  serverInfo: Signal<HelloData | null>;
}

export function ObsDcServerInfo({ serverInfo }: ObsDcServerInfoProps) {
  const info = serverInfo.value;
  if (info === null) {
    return null;
  }

  return (
    <div class="flex flex-col gap-1 rounded border border-surface-200 bg-white p-3 text-sm">
      <div class="font-medium text-slate-800">Server Info</div>
      <div class="text-slate-800">
        OBS Studio: <span class="text-slate-800">{info.obsStudioVersion}</span>
      </div>
      <div class="text-slate-800">
        obs-websocket: <span class="text-slate-800">{info.obsWebSocketVersion}</span>
      </div>
      <div class="text-slate-800">
        RPC Version: <span class="text-slate-800">{info.rpcVersion}</span>
      </div>
      <div class="text-slate-800">
        Auth: <span class="text-slate-800">{info.authentication ? "required" : "none"}</span>
      </div>
    </div>
  );
}
