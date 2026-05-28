import { signal } from "@preact/signals";
import type { Signal } from "@preact/signals";
import {
  OpCode,
  EventSubscription,
  parseServerMessage,
  serializeClientMessage,
} from "./protocol.ts";
import type { HelloData, EventData, RequestResponseData } from "./protocol.ts";
import { generateAuthenticationString } from "./auth.ts";

export type ObsDcConnectionState = "disconnected" | "connecting" | "authenticating" | "connected";

export interface ObsDcLogEntry {
  readonly timestamp: number;
  readonly level: "info" | "warn" | "error";
  readonly message: string;
}

export interface ObsDcConnectionConfig {
  readonly url: string;
  readonly password: string;
}

export interface ObsDcClientState {
  readonly connectionState: Signal<ObsDcConnectionState>;
  readonly serverInfo: Signal<HelloData | null>;
  readonly events: Signal<readonly EventData[]>;
  readonly logs: Signal<readonly ObsDcLogEntry[]>;
  readonly lastError: Signal<string | null>;
}

export interface PendingRequest {
  readonly resolve: (data: RequestResponseData) => void;
  readonly reject: (error: Error) => void;
}

export interface ObsDcClient {
  readonly state: ObsDcClientState;
  readonly connect: (config: ObsDcConnectionConfig) => void;
  readonly disconnect: () => void;
  readonly sendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

export function createObsDcClient(): ObsDcClient {
  const connectionState = signal<ObsDcConnectionState>("disconnected");
  const serverInfo = signal<HelloData | null>(null);
  const events = signal<readonly EventData[]>([]);
  const logs = signal<readonly ObsDcLogEntry[]>([]);
  const lastError = signal<string | null>(null);

  let websocket: WebSocket | null = null;
  let nextRequestId = 1;
  const pendingRequests = new Map<string, PendingRequest>();

  function addLog(level: ObsDcLogEntry["level"], message: string): void {
    const entry: ObsDcLogEntry = { timestamp: Date.now(), level, message };
    logs.value = [...logs.value, entry];
  }

  function handleMessage(event: MessageEvent): void {
    const raw = event.data as string;
    addLog("info", `Received: ${raw}`);

    try {
      const message = parseServerMessage(raw);

      switch (message.op) {
        case OpCode.Hello: {
          handleHello(message.d);
          break;
        }
        case OpCode.Identified: {
          connectionState.value = "connected";
          addLog("info", `Identified (RPC version: ${message.d.negotiatedRpcVersion})`);
          break;
        }
        case OpCode.Event: {
          events.value = [...events.value, message.d];
          addLog(
            "info",
            `Event: ${message.d.eventType}${message.d.eventData ? ` ${JSON.stringify(message.d.eventData)}` : ""}`,
          );
          break;
        }
        case OpCode.RequestResponse: {
          handleRequestResponse(message.d);
          break;
        }
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      addLog("error", `Failed to parse message: ${errorMessage}`);
    }
  }

  function handleHello(hello: HelloData): void {
    serverInfo.value = hello;
    addLog(
      "info",
      `Hello: OBS ${hello.obsStudioVersion}, obs-websocket ${hello.obsWebSocketVersion}, RPC v${hello.rpcVersion}`,
    );

    if (hello.authentication !== undefined) {
      connectionState.value = "authenticating";
      void handleAuthentication(hello);
    } else {
      sendIdentify();
    }
  }

  async function handleAuthentication(hello: HelloData): Promise<void> {
    if (hello.authentication === undefined) {
      return;
    }
    const { challenge, salt } = hello.authentication;
    const password = currentPassword;

    try {
      const authString = await generateAuthenticationString(password, salt, challenge);
      sendIdentify(authString);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      addLog("error", `Failed to generate authentication string: ${errorMessage}`);
      lastError.value = errorMessage;
      disconnect();
    }
  }

  function sendIdentify(authentication?: string): void {
    if (websocket === null || websocket.readyState !== WebSocket.OPEN) {
      return;
    }
    const message = serializeClientMessage({
      op: OpCode.Identify,
      d: {
        rpcVersion: 1,
        authentication,
        eventSubscriptions: EventSubscription.All,
      },
    });
    addLog("info", `Sent: ${message}`);
    websocket.send(message);
  }

  function handleRequestResponse(data: RequestResponseData): void {
    const pending = pendingRequests.get(data.requestId);
    if (pending !== undefined) {
      pendingRequests.delete(data.requestId);
      pending.resolve(data);
    }
    if (data.requestStatus.result) {
      addLog("info", `Response: ${data.requestType} success`);
    } else {
      addLog(
        "error",
        `Response: ${data.requestType} failed (code=${data.requestStatus.code}${data.requestStatus.comment ? `, ${data.requestStatus.comment}` : ""})`,
      );
    }
  }

  let currentPassword = "";

  function connect(config: ObsDcConnectionConfig): void {
    if (websocket !== null) {
      disconnect();
    }

    currentPassword = config.password;
    connectionState.value = "connecting";
    lastError.value = null;
    events.value = [];
    addLog("info", `Connecting: ${config.url}`);

    const ws = new WebSocket(config.url, ["obswebsocket.json"]);
    websocket = ws;

    ws.addEventListener("open", () => {
      addLog("info", "WebSocket connected");
    });

    ws.addEventListener("message", handleMessage);

    ws.addEventListener("error", () => {
      addLog("error", "WebSocket error");
      lastError.value = "WebSocket connection error";
    });

    ws.addEventListener("close", (event) => {
      addLog(
        "warn",
        `WebSocket closed: code=${event.code}${event.reason ? `, reason=${event.reason}` : ""}`,
      );
      connectionState.value = "disconnected";
      websocket = null;

      // 全ての保留中リクエストをリジェクトする
      for (const [requestId, pending] of pendingRequests) {
        pending.reject(new Error(`connection closed before response for ${requestId}`));
      }
      pendingRequests.clear();
    });
  }

  function disconnect(): void {
    if (websocket !== null) {
      addLog("info", "Disconnecting");
      websocket.close();
      websocket = null;
    }
    connectionState.value = "disconnected";

    for (const [requestId, pending] of pendingRequests) {
      pending.reject(new Error(`disconnected before response for ${requestId}`));
    }
    pendingRequests.clear();
  }

  async function sendRequest(
    requestType: string,
    requestData?: Record<string, unknown>,
  ): Promise<RequestResponseData> {
    if (websocket === null || websocket.readyState !== WebSocket.OPEN) {
      throw new Error("not connected");
    }
    if (connectionState.value !== "connected") {
      throw new Error("not identified");
    }

    const requestId = String(nextRequestId);
    nextRequestId += 1;

    const message = serializeClientMessage({
      op: OpCode.Request,
      d: {
        requestType,
        requestId,
        requestData,
      },
    });

    addLog("info", `Sent: ${message}`);
    websocket.send(message);

    return new Promise<RequestResponseData>((resolve, reject) => {
      pendingRequests.set(requestId, { resolve, reject });
    });
  }

  return {
    state: {
      connectionState,
      serverInfo,
      events,
      logs,
      lastError,
    },
    connect,
    disconnect,
    sendRequest,
  };
}
