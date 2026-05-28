import { useComputed } from "@preact/signals";
import { ConnectionPanel } from "../components/ConnectionPanel.tsx";
import { ConnectionStatus } from "../components/ConnectionStatus.tsx";
import { DataChannelStatus } from "../components/DataChannelStatus.tsx";
import { VideoDisplay } from "../components/VideoDisplay.tsx";
import { LogViewer } from "../components/LogViewer.tsx";
import { useP2PClient } from "../context/P2PClientProvider.tsx";
import type { ObsDcConnectionState } from "../obsdc/client.ts";

export function P2PPage() {
  const client = useP2PClient();

  // P2P の接続状態を ObsDcConnectionState にマッピングする
  const obsdcConnectionState = useComputed((): ObsDcConnectionState => {
    const connState = client.state.connectionState.value;
    const dcState = client.state.dataChannelState.value.obsdc;
    if (connState === "connected" && dcState === "open") {
      return "connected";
    }
    if (connState === "connecting" || connState === "bootstrapping") {
      return "connecting";
    }
    return "disconnected";
  });

  return (
    <div class="flex flex-1 flex-col overflow-hidden">
      <div class="flex flex-1 overflow-hidden">
        <aside class="flex w-96 flex-col gap-6 border-r border-surface-200 bg-white p-4">
          <ConnectionPanel
            connectionState={client.state.connectionState}
            onConnect={async (config) => client.connect(config)}
            onDisconnect={() => {
              client.disconnect();
            }}
          />
          <ConnectionStatus
            connectionState={client.state.connectionState}
            closeMessage={client.state.closeMessage}
            lastError={client.state.lastError}
          />
          <DataChannelStatus dataChannelState={client.state.dataChannelState} />
        </aside>
        <main class="flex-1 bg-surface-50 p-4">
          <div class="h-full rounded-md border border-surface-200 bg-white shadow-sm">
            <VideoDisplay tracks={client.state.tracks} />
          </div>
        </main>
      </div>
      <div class="border-t border-surface-200 p-4">
        <LogViewer
          obsdcConnectionState={obsdcConnectionState}
          events={client.state.events}
          onSendRequest={async (requestType, requestData) =>
            client.sendRequest(requestType, requestData)
          }
        />
      </div>
    </div>
  );
}
