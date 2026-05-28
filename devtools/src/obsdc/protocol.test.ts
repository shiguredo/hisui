import { test, assert } from "vite-plus/test";
import type { HelloData, EventData, RequestResponseData, ServerMessage } from "./protocol.ts";
import {
  OpCode,
  EventSubscription,
  parseServerMessage,
  serializeClientMessage,
} from "./protocol.ts";

function asHelloMessage(message: ServerMessage): HelloData {
  assert.strictEqual(message.op, OpCode.Hello);
  return (message as Extract<ServerMessage, { op: typeof OpCode.Hello }>).d;
}

function asEventMessage(message: ServerMessage): EventData {
  assert.strictEqual(message.op, OpCode.Event);
  return (message as Extract<ServerMessage, { op: typeof OpCode.Event }>).d;
}

function asRequestResponseMessage(message: ServerMessage): RequestResponseData {
  assert.strictEqual(message.op, OpCode.RequestResponse);
  return (message as Extract<ServerMessage, { op: typeof OpCode.RequestResponse }>).d;
}

test("OpCode の値が仕様通りである", () => {
  assert.strictEqual(OpCode.Hello, 0);
  assert.strictEqual(OpCode.Identify, 1);
  assert.strictEqual(OpCode.Identified, 2);
  assert.strictEqual(OpCode.Reidentify, 3);
  assert.strictEqual(OpCode.Event, 5);
  assert.strictEqual(OpCode.Request, 6);
  assert.strictEqual(OpCode.RequestResponse, 7);
  assert.strictEqual(OpCode.RequestBatch, 8);
  assert.strictEqual(OpCode.RequestBatchResponse, 9);
});

test("EventSubscription.All は下位 12 ビット全て立つ", () => {
  assert.strictEqual(EventSubscription.All, 4095);
});

test("EventSubscription の高ボリュームイベントはビット 16 以降", () => {
  assert.strictEqual(EventSubscription.InputVolumeMeters, 1 << 16);
  assert.strictEqual(EventSubscription.InputActiveStateChanged, 1 << 17);
  assert.strictEqual(EventSubscription.InputShowStateChanged, 1 << 18);
  assert.strictEqual(EventSubscription.SceneItemTransformChanged, 1 << 19);
});

test("parseServerMessage は Hello メッセージをパースできる", () => {
  const raw = JSON.stringify({
    op: 0,
    d: {
      obsStudioVersion: "30.2.2",
      obsWebSocketVersion: "5.5.2",
      rpcVersion: 1,
      authentication: {
        challenge: "abc123",
        salt: "def456",
      },
    },
  });
  const message = parseServerMessage(raw);
  assert.strictEqual(asHelloMessage(message).obsStudioVersion, "30.2.2");
});

test("parseServerMessage は認証なし Hello をパースできる", () => {
  const raw = JSON.stringify({
    op: 0,
    d: {
      obsStudioVersion: "30.2.2",
      obsWebSocketVersion: "5.5.2",
      rpcVersion: 1,
    },
  });
  const message = parseServerMessage(raw);
  assert.isUndefined(asHelloMessage(message).authentication);
});

test("parseServerMessage は Identified メッセージをパースできる", () => {
  const raw = JSON.stringify({
    op: 2,
    d: { negotiatedRpcVersion: 1 },
  });
  const message = parseServerMessage(raw);
  assert.strictEqual(message.op, OpCode.Identified);
});

test("parseServerMessage は Event メッセージをパースできる", () => {
  const raw = JSON.stringify({
    op: 5,
    d: {
      eventType: "CurrentProgramSceneChanged",
      eventIntent: 4,
      eventData: { sceneName: "Scene 1" },
    },
  });
  const message = parseServerMessage(raw);
  assert.strictEqual(asEventMessage(message).eventType, "CurrentProgramSceneChanged");
});

test("parseServerMessage は RequestResponse をパースできる", () => {
  const raw = JSON.stringify({
    op: 7,
    d: {
      requestType: "GetSceneList",
      requestId: "test-1",
      requestStatus: { result: true, code: 100 },
      responseData: { scenes: [] },
    },
  });
  const message = parseServerMessage(raw);
  assert.isTrue(asRequestResponseMessage(message).requestStatus.result);
});

test("parseServerMessage は不正な JSON でエラーになる", () => {
  assert.throws(() => parseServerMessage("not json"), SyntaxError);
});

test("parseServerMessage は op フィールドがない場合エラーになる", () => {
  assert.throws(() => parseServerMessage(JSON.stringify({ d: {} })));
});

test("parseServerMessage は d フィールドがない場合エラーになる", () => {
  assert.throws(() => parseServerMessage(JSON.stringify({ op: 0 })));
});

test("serializeClientMessage は Identify メッセージをシリアライズできる", () => {
  const message = serializeClientMessage({
    op: OpCode.Identify,
    d: { rpcVersion: 1, authentication: "auth-string", eventSubscriptions: 33 },
  });
  const parsed = JSON.parse(message);
  assert.strictEqual(parsed.op, 1);
  assert.strictEqual(parsed.d.rpcVersion, 1);
  assert.strictEqual(parsed.d.authentication, "auth-string");
});

test("serializeClientMessage は Request メッセージをシリアライズできる", () => {
  const message = serializeClientMessage({
    op: OpCode.Request,
    d: {
      requestType: "GetSceneList",
      requestId: "req-1",
    },
  });
  const parsed = JSON.parse(message);
  assert.strictEqual(parsed.op, 6);
  assert.strictEqual(parsed.d.requestType, "GetSceneList");
});
