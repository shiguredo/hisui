import { signal } from "@preact/signals";
import type { Signal } from "@preact/signals";
import type {
  ConnectionState,
  DataChannelState,
  CloseMessage,
  BootstrapConfig,
  LogEntry,
  LogLevel,
  LogCategory,
} from "./types.ts";
import { parseServerMessage, serializeClientMessage } from "./signaling.ts";
import {
  OpCode,
  parseServerMessage as parseObsDcMessage,
  serializeClientMessage as serializeObsDcMessage,
} from "../obsdc/protocol.ts";
import type { EventData, RequestResponseData } from "../obsdc/protocol.ts";

export interface P2PClientState {
  readonly connectionState: Signal<ConnectionState>;
  readonly dataChannelState: Signal<DataChannelState>;
  readonly tracks: Signal<readonly MediaStreamTrack[]>;
  readonly receivers: Signal<readonly RTCRtpReceiver[]>;
  readonly closeMessage: Signal<CloseMessage | null>;
  readonly lastError: Signal<Error | null>;
  readonly logs: Signal<readonly LogEntry[]>;
  readonly events: Signal<readonly EventData[]>;
}

export interface P2PClient {
  readonly state: P2PClientState;
  readonly connect: (config: BootstrapConfig) => Promise<void>;
  readonly disconnect: () => void;
  readonly dispose: () => void;
  readonly sendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
  readonly getPeerConnectionStats: () => Promise<RTCStatsReport | null>;
}

const INITIAL_DATA_CHANNEL_STATE: DataChannelState = {
  signaling: "not-created",
  obsdc: "not-created",
};

export function supportsAlwaysNegotiateDataChannels(): boolean {
  try {
    const pc = new RTCPeerConnection({ alwaysNegotiateDataChannels: true });
    const config = pc.getConfiguration();
    pc.close();
    return "alwaysNegotiateDataChannels" in config;
  } catch {
    return false;
  }
}

// host candidate は即座に収集されるため、短いタイムアウトで十分
async function waitForIceGathering(pc: RTCPeerConnection, timeoutMs = 100): Promise<void> {
  if (pc.iceGatheringState === "complete") {
    return;
  }
  let onIceCandidate: ((event: RTCPeerConnectionIceEvent) => void) | undefined;
  try {
    await Promise.race([
      new Promise<void>((resolve) => {
        onIceCandidate = (event: RTCPeerConnectionIceEvent): void => {
          if (event.candidate === null) {
            resolve();
          }
        };
        pc.addEventListener("icecandidate", onIceCandidate);
      }),
      new Promise<void>((resolve) => {
        setTimeout(resolve, timeoutMs);
      }),
    ]);
  } finally {
    if (onIceCandidate !== undefined) {
      pc.removeEventListener("icecandidate", onIceCandidate);
    }
  }
}

export function createP2PClient(): P2PClient {
  const connectionState = signal<ConnectionState>("idle");
  const dataChannelState = signal<DataChannelState>(INITIAL_DATA_CHANNEL_STATE);
  const tracks = signal<readonly MediaStreamTrack[]>([]);
  const receivers = signal<readonly RTCRtpReceiver[]>([]);
  const closeMessage = signal<CloseMessage | null>(null);
  const lastError = signal<Error | null>(null);
  const logs = signal<readonly LogEntry[]>([]);
  const events = signal<readonly EventData[]>([]);

  let peerConnection: RTCPeerConnection | null = null;
  let signalingChannel: RTCDataChannel | null = null;
  let obsdcChannel: RTCDataChannel | null = null;
  let nextRequestId = 1;
  const pendingRequests = new Map<
    string,
    { resolve: (data: RequestResponseData) => void; reject: (error: Error) => void }
  >();

  function addLog(category: LogCategory, level: LogLevel, message: string): void {
    const entry: LogEntry = { timestamp: Date.now(), level, category, message };
    logs.value = [...logs.value, entry];
  }

  function resetState(): void {
    connectionState.value = "idle";
    dataChannelState.value = INITIAL_DATA_CHANNEL_STATE;
    tracks.value = [];
    receivers.value = [];
    closeMessage.value = null;
    lastError.value = null;
    logs.value = [];
    events.value = [];
    nextRequestId = 1;
  }

  function stopAndRemoveAllTracks(): void {
    for (const track of tracks.value) {
      track.stop();
    }
    tracks.value = [];
    receivers.value = [];
  }

  function rejectAllPendingRequests(): void {
    for (const [requestId, pending] of pendingRequests) {
      pending.reject(new Error(`connection closed before response for ${requestId}`));
    }
    pendingRequests.clear();
  }

  function closePeerConnection(): void {
    stopAndRemoveAllTracks();
    rejectAllPendingRequests();
    if (peerConnection !== null) {
      peerConnection.ontrack = null;
      peerConnection.ondatachannel = null;
      peerConnection.onconnectionstatechange = null;
      peerConnection.close();
      peerConnection = null;
    }
    signalingChannel = null;
    obsdcChannel = null;
  }

  function handleSignalingMessage(event: MessageEvent): void {
    const raw = event.data as string;
    addLog("signaling", "info", `Received: ${raw}`);
    try {
      const message = parseServerMessage(raw);
      switch (message.type) {
        case "offer": {
          addLog("signaling", "info", "Received offer, starting re-negotiation");
          void handleReNegotiation(message.sdp);
          break;
        }
        case "close": {
          addLog(
            "signaling",
            "warn",
            `Received close: code=${message.code}, reason=${message.reason}`,
          );
          closeMessage.value = message;
          closePeerConnection();
          connectionState.value = "closed";
          break;
        }
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      addLog("signaling", "error", `Failed to parse message: ${errorMessage}`);
      lastError.value = error instanceof Error ? error : new Error(String(error));
    }
  }

  // renegotiation 後に不要になったトラックを除去する
  //
  // サーバーが pc.remove_track() した後の renegotiation では、
  // transceiver の currentDirection が "inactive" になるが
  // track.readyState は "live" のまま残る。
  // currentDirection で受信中かどうかを判定する。
  function syncTracksWithReceivers(pc: RTCPeerConnection): void {
    const receivingTrackIds = new Set<string>();
    for (const transceiver of pc.getTransceivers()) {
      const dir = transceiver.currentDirection;
      // ブラウザ側から見て受信中 ("recvonly" or "sendrecv") のトラックだけ残す
      if (dir === "recvonly" || dir === "sendrecv") {
        receivingTrackIds.add(transceiver.receiver.track.id);
      }
    }
    const removed = tracks.value.filter((t) => !receivingTrackIds.has(t.id));
    if (removed.length > 0) {
      for (const t of removed) {
        addLog("pc", "info", `Track pruned after renegotiation: kind=${t.kind}, id=${t.id}`);
        t.stop();
      }
      tracks.value = tracks.value.filter((t) => receivingTrackIds.has(t.id));
      receivers.value = receivers.value.filter((r) => receivingTrackIds.has(r.track.id));
    }
  }

  async function handleReNegotiation(sdp: string): Promise<void> {
    if (peerConnection === null || signalingChannel === null) {
      return;
    }
    const pc = peerConnection;
    const channel = signalingChannel;
    try {
      addLog("pc", "info", "setRemoteDescription(offer)");
      await pc.setRemoteDescription({ type: "offer", sdp });
      addLog("pc", "info", "createAnswer()");
      const answer = await pc.createAnswer();
      addLog("pc", "info", "setLocalDescription(answer)");
      await pc.setLocalDescription(answer);
      if (pc.localDescription === null) {
        return;
      }
      const answerMessage = serializeClientMessage({
        type: "answer",
        sdp: pc.localDescription.sdp,
      });
      addLog("signaling", "info", `Sent: ${answerMessage}`);
      channel.send(answerMessage);
      syncTracksWithReceivers(pc);
    } catch (error: unknown) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      addLog("pc", "error", `Re-negotiation failed: ${errorMessage}`);
      lastError.value = error instanceof Error ? error : new Error(String(error));
    }
  }

  function handleObswsMessage(event: MessageEvent): void {
    const raw = event.data as string;
    addLog("obsdc", "info", `Received: ${raw}`);
    try {
      const message = parseObsDcMessage(raw);
      switch (message.op) {
        case OpCode.Hello:
        case OpCode.Identified: {
          addLog("obsdc", "info", `Unhandled OpCode: ${message.op}`);
          break;
        }
        case OpCode.RequestResponse: {
          if (message.d.requestStatus.result) {
            addLog("obsdc", "info", `Response: ${message.d.requestType} success`);
          } else {
            addLog(
              "obsdc",
              "error",
              `Response: ${message.d.requestType} failed (code=${message.d.requestStatus.code}${message.d.requestStatus.comment ? `, ${message.d.requestStatus.comment}` : ""})`,
            );
          }
          const pending = pendingRequests.get(message.d.requestId);
          if (pending !== undefined) {
            pendingRequests.delete(message.d.requestId);
            pending.resolve(message.d);
          }
          break;
        }
        case OpCode.Event: {
          events.value = [...events.value, message.d];
          addLog(
            "obsdc",
            "info",
            `Event: ${message.d.eventType}${message.d.eventData ? ` ${JSON.stringify(message.d.eventData)}` : ""}`,
          );
          break;
        }
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      addLog("obsdc", "error", `Failed to parse message: ${errorMessage}`);
    }
  }

  async function sendRequest(
    requestType: string,
    requestData?: Record<string, unknown>,
  ): Promise<RequestResponseData> {
    if (obsdcChannel === null) {
      addLog("obsdc", "error", "OBS WebSocket DataChannel is not open");
      throw new Error("obsdc datachannel is not open");
    }
    if (obsdcChannel.readyState !== "open") {
      addLog("obsdc", "error", "OBS WebSocket DataChannel is not open");
      throw new Error("obsdc datachannel is not open");
    }
    const requestId = String(nextRequestId);
    nextRequestId += 1;
    const message = serializeObsDcMessage({
      op: OpCode.Request,
      d: {
        requestType,
        requestId,
        requestData,
      },
    });
    addLog("obsdc", "info", `Sent: ${message}`);
    obsdcChannel.send(message);

    return new Promise<RequestResponseData>((resolve, reject) => {
      pendingRequests.set(requestId, { resolve, reject });
    });
  }

  function bindDataChannel(
    channel: RTCDataChannel,
    label: "signaling" | "obsdc",
    onMessage: (event: MessageEvent) => void,
  ): void {
    channel.addEventListener("message", onMessage);
    channel.addEventListener("open", () => {
      addLog(label, "info", "DataChannel opened");
      dataChannelState.value = {
        ...dataChannelState.value,
        [label]: channel.readyState,
      };
    });
    channel.addEventListener("close", () => {
      addLog(label, "info", "DataChannel closed");
      dataChannelState.value = {
        ...dataChannelState.value,
        [label]: channel.readyState,
      };
    });
    dataChannelState.value = {
      ...dataChannelState.value,
      [label]: channel.readyState,
    };
  }

  function handleDataChannel(event: RTCDataChannelEvent): void {
    const { channel } = event;
    addLog("pc", "info", `DataChannel "${channel.label}" received (state=${channel.readyState})`);
    switch (channel.label) {
      case "signaling": {
        signalingChannel = channel;
        bindDataChannel(channel, "signaling", handleSignalingMessage);
        break;
      }
      case "obsdc": {
        obsdcChannel = channel;
        bindDataChannel(channel, "obsdc", handleObswsMessage);
        break;
      }
      default: {
        addLog("pc", "warn", `Unknown DataChannel "${channel.label}" ignored`);
        break;
      }
    }
  }

  // ページ離脱時にサーバーへ切断を通知する
  function handleBeforeUnload(): void {
    if (signalingChannel?.readyState === "open") {
      const disconnectMessage = serializeClientMessage({ type: "disconnect" });
      signalingChannel.send(disconnectMessage);
    }
    closePeerConnection();
  }

  async function connect(config: BootstrapConfig): Promise<void> {
    connectionState.value = "bootstrapping";
    lastError.value = null;
    closeMessage.value = null;
    window.addEventListener("beforeunload", handleBeforeUnload);

    addLog("pc", "info", `Starting bootstrap: ${config.bootstrapUrl}`);
    addLog("pc", "info", `DataChannel only: ${config.dataChannelOnly}`);

    try {
      const pc = new RTCPeerConnection({
        alwaysNegotiateDataChannels: config.dataChannelOnly ? true : undefined,
        iceServers: config.iceServers ? [...config.iceServers] : undefined,
      });
      peerConnection = pc;

      if (config.dataChannelOnly) {
        addLog(
          "pc",
          "info",
          "Created RTCPeerConnection (alwaysNegotiateDataChannels: SDP contains m=application only)",
        );
      } else {
        pc.createDataChannel("dummy");
        addLog(
          "pc",
          "info",
          "Created RTCPeerConnection (createDataChannel fallback: dummy DataChannel created)",
        );
      }

      pc.ondatachannel = handleDataChannel;

      pc.ontrack = (event) => {
        const { track } = event;
        const { receiver } = event;
        addLog("pc", "info", `Track received: kind=${track.kind}, id=${track.id}`);
        tracks.value = [...tracks.value, track];
        receivers.value = [...receivers.value, receiver];

        function removeTrack(): void {
          addLog("pc", "info", `Track removed: kind=${track.kind}, id=${track.id}`);
          tracks.value = tracks.value.filter((t) => t.id !== track.id);
          receivers.value = receivers.value.filter((r) => r !== receiver);
        }

        track.addEventListener("ended", removeTrack);
        track.addEventListener("mute", () => {
          addLog("pc", "info", `Track muted: kind=${track.kind}, id=${track.id}`);
        });
        track.addEventListener("unmute", () => {
          addLog("pc", "info", `Track unmuted: kind=${track.kind}, id=${track.id}`);
        });

        for (const stream of event.streams) {
          stream.addEventListener("removetrack", (removeEvent) => {
            addLog(
              "pc",
              "info",
              `Track removed from stream: kind=${removeEvent.track.kind}, id=${removeEvent.track.id}`,
            );
            removeEvent.track.stop();
            tracks.value = tracks.value.filter((t) => t.id !== removeEvent.track.id);
            receivers.value = receivers.value.filter((r) => r.track.id !== removeEvent.track.id);
          });
        }
      };

      pc.onconnectionstatechange = () => {
        addLog("pc", "info", `State changed: ${pc.connectionState}`);
        switch (pc.connectionState) {
          case "connected": {
            connectionState.value = "connected";
            break;
          }
          case "connecting":
          case "new": {
            break;
          }
          case "disconnected":
          case "failed":
          case "closed": {
            stopAndRemoveAllTracks();
            connectionState.value = "closed";
            break;
          }
        }
      };

      addLog("pc", "info", "createOffer()");
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);

      addLog("pc", "info", "Waiting for ICE gathering");
      await waitForIceGathering(pc);
      addLog("pc", "info", "ICE gathering complete");

      const offerSdp = pc.localDescription!.sdp;
      addLog("pc", "info", `POST ${config.bootstrapUrl} (Content-Type: application/sdp)`);
      addLog("pc", "info", `offer SDP:\n${offerSdp}`);
      const response = await fetch(config.bootstrapUrl, {
        method: "POST",
        headers: { "Content-Type": "application/sdp" },
        body: offerSdp,
      });

      addLog("pc", "info", `Bootstrap response: ${response.status} ${response.statusText}`);

      if (response.status !== 201) {
        throw new Error(`bootstrap failed with status: ${response.status}`);
      }

      const answerSdp = await response.text();
      addLog("pc", "info", `answer SDP:\n${answerSdp}`);

      addLog("pc", "info", "setRemoteDescription(answer)");
      await pc.setRemoteDescription({ type: "answer", sdp: answerSdp });

      connectionState.value = "connecting";
      addLog("pc", "info", "Connecting (waiting for WebRTC handshake)");

      // 接続が完了しない場合はタイムアウトで切断する
      const connectionTimeoutMs = 5000;
      setTimeout(() => {
        if (connectionState.value === "connecting") {
          addLog("pc", "error", "Connection timed out");
          lastError.value = new Error("connection timeout");
          closePeerConnection();
          connectionState.value = "closed";
        }
      }, connectionTimeoutMs);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      addLog("pc", "error", `Bootstrap failed: ${errorMessage}`);
      lastError.value = error instanceof Error ? error : new Error(String(error));
      closePeerConnection();
      connectionState.value = "closed";
    }
  }

  function disconnect(): void {
    addLog("pc", "info", "Disconnecting");
    connectionState.value = "disconnecting";
    window.removeEventListener("beforeunload", handleBeforeUnload);
    if (signalingChannel?.readyState === "open") {
      const disconnectMessage = serializeClientMessage({ type: "disconnect" });
      addLog("signaling", "info", `Sent: ${disconnectMessage}`);
      signalingChannel.send(disconnectMessage);
    }
    closePeerConnection();
    connectionState.value = "closed";
    addLog("pc", "info", "Disconnected");
  }

  async function getPeerConnectionStats(): Promise<RTCStatsReport | null> {
    if (peerConnection === null) {
      return null;
    }
    try {
      return await peerConnection.getStats();
    } catch {
      return null;
    }
  }

  function dispose(): void {
    closePeerConnection();
    resetState();
  }

  return {
    state: {
      connectionState,
      dataChannelState,
      tracks,
      receivers,
      closeMessage,
      lastError,
      logs,
      events,
    },
    connect,
    disconnect,
    dispose,
    sendRequest,
    getPeerConnectionStats,
  };
}
