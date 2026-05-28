import { test, assert } from "vite-plus/test";
import { createObsDcClient } from "./client.ts";

test("createObsDcClient は初期状態が正しい", () => {
  const client = createObsDcClient();
  assert.strictEqual(client.state.connectionState.value, "disconnected");
  assert.isNull(client.state.serverInfo.value);
  assert.deepStrictEqual(client.state.events.value, []);
  assert.deepStrictEqual(client.state.logs.value, []);
  assert.isNull(client.state.lastError.value);
});

test("sendRequest は未接続時にリジェクトされる", async () => {
  const client = createObsDcClient();
  try {
    await client.sendRequest("GetVersion");
    assert.fail("リジェクトされるべき");
  } catch (error) {
    assert.instanceOf(error, Error);
    assert.include(error.message, "not connected");
  }
});

test("createObsDcClient のインターフェースが正しい", () => {
  const client = createObsDcClient();
  assert.isFunction(client.connect);
  assert.isFunction(client.disconnect);
  assert.isFunction(client.sendRequest);
  assert.isObject(client.state);
});
