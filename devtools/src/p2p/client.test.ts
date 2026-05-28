import { test, assert } from "vite-plus/test";
import { createP2PClient } from "./client.ts";

test("client モジュールがインポートできる", async () => {
  const module = await import("./client.ts");
  assert.isFunction(module.createP2PClient);
});

test("createP2PClient は初期状態で idle を返す", () => {
  const client = createP2PClient();
  assert.strictEqual(client.state.connectionState.value, "idle");
});

test("createP2PClient の DataChannel 初期状態は not-created", () => {
  const client = createP2PClient();
  assert.deepStrictEqual(client.state.dataChannelState.value, {
    signaling: "not-created",
    obsdc: "not-created",
  });
});

test("createP2PClient の tracks 初期状態は空配列", () => {
  const client = createP2PClient();
  assert.deepStrictEqual(client.state.tracks.value, []);
});

test("createP2PClient の closeMessage 初期状態は null", () => {
  const client = createP2PClient();
  assert.isNull(client.state.closeMessage.value);
});

test("createP2PClient の lastError 初期状態は null", () => {
  const client = createP2PClient();
  assert.isNull(client.state.lastError.value);
});

test("createP2PClient の logs 初期状態は空配列", () => {
  const client = createP2PClient();
  assert.deepStrictEqual(client.state.logs.value, []);
});

test("dispose は状態を初期値にリセットする", () => {
  const client = createP2PClient();
  client.dispose();
  assert.strictEqual(client.state.connectionState.value, "idle");
  assert.deepStrictEqual(client.state.dataChannelState.value, {
    signaling: "not-created",
    obsdc: "not-created",
  });
  assert.deepStrictEqual(client.state.tracks.value, []);
  assert.deepStrictEqual(client.state.receivers.value, []);
  assert.isNull(client.state.closeMessage.value);
  assert.isNull(client.state.lastError.value);
  assert.deepStrictEqual(client.state.logs.value, []);
  assert.deepStrictEqual(client.state.events.value, []);
});

test("createP2PClient の receivers 初期状態は空配列", () => {
  const client = createP2PClient();
  assert.deepStrictEqual(client.state.receivers.value, []);
});

test("createP2PClient は getPeerConnectionStats メソッドを持つ", () => {
  const client = createP2PClient();
  assert.isFunction(client.getPeerConnectionStats);
});

test("接続前の getPeerConnectionStats は null を返す", async () => {
  const client = createP2PClient();
  const result = await client.getPeerConnectionStats();
  assert.isNull(result);
});

test("createP2PClient は sendRequest メソッドを持つ", () => {
  const client = createP2PClient();
  assert.isFunction(client.sendRequest);
});

test("DataChannel 未接続時の sendRequest は reject する", async () => {
  const client = createP2PClient();
  try {
    await client.sendRequest("GetVersion");
    assert.fail("sendRequest should reject");
  } catch (error) {
    assert.instanceOf(error, Error);
  }
});

test("createP2PClient の events 初期状態は空配列", () => {
  const client = createP2PClient();
  assert.deepStrictEqual(client.state.events.value, []);
});
