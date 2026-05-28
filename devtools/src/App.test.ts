import { test, assert } from "vite-plus/test";

test("App モジュールがインポートできる", async () => {
  const module = await import("./App.tsx");
  assert.isFunction(module.App);
});

test("P2PPage モジュールがインポートできる", async () => {
  const module = await import("./pages/P2PPage.tsx");
  assert.isFunction(module.P2PPage);
});

test("DebugPage モジュールがインポートできる", async () => {
  const module = await import("./pages/DebugPage.tsx");
  assert.isFunction(module.DebugPage);
});
