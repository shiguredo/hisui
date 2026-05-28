import { test, assert } from "vite-plus/test";
import { parseServerMessage, serializeClientMessage } from "./signaling.ts";

// parseServerMessage

test("parseServerMessage は有効な offer メッセージをパースする", () => {
  const result = parseServerMessage(String.raw`{"type":"offer","sdp":"v=0\r\n"}`);
  assert.deepStrictEqual(result, { type: "offer", sdp: "v=0\r\n" });
});

test("parseServerMessage は有効な close メッセージをパースする", () => {
  const result = parseServerMessage('{"type":"close","code":"timeout","reason":"timed out"}');
  assert.deepStrictEqual(result, {
    type: "close",
    code: "timeout",
    reason: "timed out",
  });
});

test("parseServerMessage は不正な JSON でエラーを投げる", () => {
  assert.throws(
    () => parseServerMessage("not json"),
    /failed to parse server message: invalid JSON/u,
  );
});

test("parseServerMessage は type フィールドがない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage(String.raw`{"sdp":"v=0\r\n"}`),
    /failed to parse server message: missing type field/u,
  );
});

test("parseServerMessage は type フィールドが文字列でない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":42}'),
    /failed to parse server message: missing type field/u,
  );
});

test("parseServerMessage は未知の type でエラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"unknown"}'),
    /unknown server message type: unknown/u,
  );
});

test("parseServerMessage は offer に sdp がない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"offer"}'),
    /missing sdp field in offer message/u,
  );
});

test("parseServerMessage は offer の sdp が文字列でない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"offer","sdp":42}'),
    /missing sdp field in offer message/u,
  );
});

test("parseServerMessage は close に code がない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"close","reason":"test"}'),
    /missing code field in close message/u,
  );
});

test("parseServerMessage は close に reason がない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"close","code":"timeout"}'),
    /missing reason field in close message/u,
  );
});

test("parseServerMessage は close の reason が文字列でない場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"close","code":"timeout","reason":42}'),
    /missing reason field in close message/u,
  );
});

test("parseServerMessage は close の code が無効な場合エラーを投げる", () => {
  assert.throws(
    () => parseServerMessage('{"type":"close","code":"invalid","reason":"test"}'),
    /unknown close code: invalid/u,
  );
});

// serializeClientMessage

test("serializeClientMessage は answer メッセージをシリアライズする", () => {
  const result = serializeClientMessage({ type: "answer", sdp: "v=0\r\n" });
  assert.strictEqual(result, String.raw`{"type":"answer","sdp":"v=0\r\n"}`);
});

test("serializeClientMessage は disconnect メッセージをシリアライズする", () => {
  const result = serializeClientMessage({ type: "disconnect" });
  assert.strictEqual(result, '{"type":"disconnect"}');
});
