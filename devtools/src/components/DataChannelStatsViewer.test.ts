import { test, assert } from "vite-plus/test";

test("DataChannelStatsViewer モジュールがインポートできる", async () => {
  const module = await import("./DataChannelStatsViewer.tsx");
  assert.isFunction(module.DataChannelStatsViewer);
});
