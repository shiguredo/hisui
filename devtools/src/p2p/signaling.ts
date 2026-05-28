import type { ServerMessage, ClientMessage } from "./types.ts";
import { isCloseCode } from "./types.ts";

export function parseServerMessage(data: string): ServerMessage {
  let parsed: unknown;
  try {
    parsed = JSON.parse(data);
  } catch {
    throw new Error("failed to parse server message: invalid JSON");
  }

  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("failed to parse server message: missing type field");
  }

  const record = parsed as Record<string, unknown>;

  if (typeof record.type !== "string") {
    throw new TypeError("failed to parse server message: missing type field");
  }

  switch (record.type) {
    case "offer": {
      if (typeof record.sdp !== "string") {
        throw new TypeError("missing sdp field in offer message");
      }
      return { type: "offer", sdp: record.sdp };
    }
    case "close": {
      if (!("code" in record) || typeof record.code !== "string") {
        throw new Error("missing code field in close message");
      }
      if (typeof record.reason !== "string") {
        throw new TypeError("missing reason field in close message");
      }
      if (!isCloseCode(record.code)) {
        throw new Error(`unknown close code: ${record.code}`);
      }
      return { type: "close", code: record.code, reason: record.reason };
    }
    default: {
      throw new Error(`unknown server message type: ${record.type}`);
    }
  }
}

export function serializeClientMessage(message: ClientMessage): string {
  return JSON.stringify(message);
}
