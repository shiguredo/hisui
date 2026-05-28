// 接続状態
export type ConnectionState =
  | "idle"
  | "bootstrapping"
  | "connecting"
  | "connected"
  | "disconnecting"
  | "closed";

// サーバーから送信される close コード
export type CloseCode =
  | "unknown-type"
  | "timeout"
  | "sdp-error"
  | "srd-error"
  | "unexpected"
  | "missing-sdp";

const CLOSE_CODES: ReadonlySet<string> = new Set<CloseCode>([
  "unknown-type",
  "timeout",
  "sdp-error",
  "srd-error",
  "unexpected",
  "missing-sdp",
]);

// サーバーメッセージ
export interface OfferMessage {
  type: "offer";
  sdp: string;
}
export interface CloseMessage {
  type: "close";
  code: CloseCode;
  reason: string;
}
export type ServerMessage = OfferMessage | CloseMessage;

// クライアントメッセージ
export interface AnswerMessage {
  type: "answer";
  sdp: string;
}
export interface DisconnectMessage {
  type: "disconnect";
}
export type ClientMessage = AnswerMessage | DisconnectMessage;

// DataChannel 状態
export interface DataChannelState {
  signaling: RTCDataChannelState | "not-created";
  obsdc: RTCDataChannelState | "not-created";
}

// Bootstrap 設定
export interface BootstrapConfig {
  bootstrapUrl: string;
  iceServers?: readonly RTCIceServer[];
  dataChannelOnly: boolean;
}

// ログレベル
export type LogLevel = "info" | "warn" | "error";

// ログカテゴリ
export type LogCategory = "pc" | "signaling" | "obsdc";

// デバッグタブ
export type DebugTab = LogCategory | "track-stats" | "datachannel-stats";

// OBS パネルタブ
export type ObsPanelTab = "obs-scenes" | "obs-sources" | "obs-stream-record";

// ビューワータブ
export type ViewerTab = DebugTab | ObsPanelTab;

// ログエントリ
export interface LogEntry {
  timestamp: number;
  level: LogLevel;
  category: LogCategory;
  message: string;
}

// 型ガード関数

export function isCloseCode(value: unknown): value is CloseCode {
  return typeof value === "string" && CLOSE_CODES.has(value);
}

export function isOfferMessage(value: unknown): value is OfferMessage {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return record.type === "offer" && typeof record.sdp === "string";
}

export function isCloseMessage(value: unknown): value is CloseMessage {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return record.type === "close" && isCloseCode(record.code) && typeof record.reason === "string";
}

export function isServerMessage(value: unknown): value is ServerMessage {
  return isOfferMessage(value) || isCloseMessage(value);
}
