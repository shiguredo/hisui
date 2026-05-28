// OBS WebSocket 5.x プロトコル定義

export const OpCode = {
  Hello: 0,
  Identify: 1,
  Identified: 2,
  Reidentify: 3,
  Event: 5,
  Request: 6,
  RequestResponse: 7,
  RequestBatch: 8,
  RequestBatchResponse: 9,
} as const;

export type OpCode = (typeof OpCode)[keyof typeof OpCode];

export const EventSubscription = {
  None: 0,
  General: 1,
  Config: 1 << 1,
  Scenes: 1 << 2,
  Inputs: 1 << 3,
  Transitions: 1 << 4,
  Filters: 1 << 5,
  Outputs: 1 << 6,
  SceneItems: 1 << 7,
  MediaInputs: 1 << 8,
  Vendors: 1 << 9,
  Ui: 1 << 10,
  Canvases: 1 << 11,
  All: (1 << 12) - 1,
  InputVolumeMeters: 1 << 16,
  InputActiveStateChanged: 1 << 17,
  InputShowStateChanged: 1 << 18,
  SceneItemTransformChanged: 1 << 19,
} as const;

export interface AuthenticationChallenge {
  readonly challenge: string;
  readonly salt: string;
}

export interface HelloData {
  readonly obsStudioVersion: string;
  readonly obsWebSocketVersion: string;
  readonly rpcVersion: number;
  readonly authentication?: AuthenticationChallenge;
}

export interface IdentifyData {
  readonly rpcVersion: number;
  readonly authentication?: string;
  readonly eventSubscriptions?: number;
}

export interface IdentifiedData {
  readonly negotiatedRpcVersion: number;
}

export interface EventData {
  readonly eventType: string;
  readonly eventIntent: number;
  readonly eventData?: Record<string, unknown>;
}

export interface RequestData {
  readonly requestType: string;
  readonly requestId: string;
  readonly requestData?: Record<string, unknown>;
}

export interface RequestStatus {
  readonly result: boolean;
  readonly code: number;
  readonly comment?: string;
}

export interface RequestResponseData {
  readonly requestType: string;
  readonly requestId: string;
  readonly requestStatus: RequestStatus;
  readonly responseData?: Record<string, unknown>;
}

export type ServerMessage =
  | { readonly op: typeof OpCode.Hello; readonly d: HelloData }
  | { readonly op: typeof OpCode.Identified; readonly d: IdentifiedData }
  | { readonly op: typeof OpCode.Event; readonly d: EventData }
  | { readonly op: typeof OpCode.RequestResponse; readonly d: RequestResponseData };

export type ClientMessage =
  | { readonly op: typeof OpCode.Identify; readonly d: IdentifyData }
  | { readonly op: typeof OpCode.Reidentify; readonly d: { readonly eventSubscriptions?: number } }
  | { readonly op: typeof OpCode.Request; readonly d: RequestData };

export function parseServerMessage(raw: string): ServerMessage {
  const parsed: unknown = JSON.parse(raw);
  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("expected object message");
  }
  const message = parsed as Record<string, unknown>;
  if (typeof message.op !== "number") {
    throw new TypeError("expected numeric op field");
  }
  if (typeof message.d !== "object" || message.d === null) {
    throw new Error("expected object d field");
  }
  return message as unknown as ServerMessage;
}

export function serializeClientMessage(message: ClientMessage): string {
  return JSON.stringify(message);
}
