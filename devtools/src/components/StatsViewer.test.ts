import { test, assert } from "vite-plus/test";

test("StatsViewer モジュールがインポートできる", async () => {
  const module = await import("./StatsViewer.tsx");
  assert.isFunction(module.StatsViewer);
});
