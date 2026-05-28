import { test, assert } from "vite-plus/test";

import {
  snapPosition,
  clampPosition,
  calculateResizedDimensions,
  calculateInitialLayout,
} from "./videoDisplayLayout.ts";

// snapPosition

test("snapPosition: コンテナの左端にスナップする", () => {
  const result = snapPosition({ x: 5, y: 100, width: 200, height: 150 }, [], 800, 600);
  assert.strictEqual(result.x, 0);
  assert.strictEqual(result.y, 100);
});

test("snapPosition: コンテナの上端にスナップする", () => {
  const result = snapPosition({ x: 100, y: 3, width: 200, height: 150 }, [], 800, 600);
  assert.strictEqual(result.x, 100);
  assert.strictEqual(result.y, 0);
});

test("snapPosition: コンテナの右端にスナップする", () => {
  const result = snapPosition({ x: 595, y: 100, width: 200, height: 150 }, [], 800, 600);
  assert.strictEqual(result.x, 600);
  assert.strictEqual(result.y, 100);
});

test("snapPosition: コンテナの下端にスナップする", () => {
  const result = snapPosition({ x: 100, y: 445, width: 200, height: 150 }, [], 800, 600);
  assert.strictEqual(result.x, 100);
  assert.strictEqual(result.y, 450);
});

test("snapPosition: 他の矩形の右辺に自分の左辺がスナップする", () => {
  const result = snapPosition(
    { x: 203, y: 100, width: 200, height: 150 },
    [{ x: 0, y: 100, width: 200, height: 150 }],
    800,
    600,
  );
  assert.strictEqual(result.x, 200);
});

test("snapPosition: 他の矩形の下辺に自分の上辺がスナップする", () => {
  const result = snapPosition(
    { x: 100, y: 155, width: 200, height: 150 },
    [{ x: 100, y: 0, width: 200, height: 150 }],
    800,
    600,
  );
  assert.strictEqual(result.y, 150);
});

test("snapPosition: 閾値を超えた場合はスナップしない", () => {
  const result = snapPosition({ x: 50, y: 50, width: 200, height: 150 }, [], 800, 600);
  assert.strictEqual(result.x, 50);
  assert.strictEqual(result.y, 50);
});

test("snapPosition: コンテナの中央にスナップする", () => {
  const result = snapPosition({ x: 297, y: 222, width: 200, height: 150 }, [], 800, 600);
  // 中央: (800-200)/2 = 300, (600-150)/2 = 225
  assert.strictEqual(result.x, 300);
  assert.strictEqual(result.y, 225);
});

// clampPosition

test("clampPosition: 範囲内の位置はそのまま返す", () => {
  const result = clampPosition(100, 100, 200, 150, 800, 600);
  assert.deepEqual(result, { x: 100, y: 100 });
});

test("clampPosition: 負の値を 0 にクランプする", () => {
  const result = clampPosition(-50, -30, 200, 150, 800, 600);
  assert.deepEqual(result, { x: 0, y: 0 });
});

test("clampPosition: 右端を超えた場合にクランプする", () => {
  const result = clampPosition(700, 100, 200, 150, 800, 600);
  assert.deepEqual(result, { x: 600, y: 100 });
});

test("clampPosition: 下端を超えた場合にクランプする", () => {
  const result = clampPosition(100, 500, 200, 150, 800, 600);
  assert.deepEqual(result, { x: 100, y: 450 });
});

test("clampPosition: 右端と下端の両方を超えた場合にクランプする", () => {
  const result = clampPosition(700, 500, 200, 150, 800, 600);
  assert.deepEqual(result, { x: 600, y: 450 });
});

// calculateResizedDimensions

test("calculateResizedDimensions: アスペクト比を維持してリサイズする", () => {
  const result = calculateResizedDimensions(320, 40, 40, 16 / 9, 160, 1920);
  assert.strictEqual(result.width, 360);
  assert.approximately(result.height, 360 / (16 / 9), 0.01);
});

test("calculateResizedDimensions: 最小幅より小さくならない", () => {
  const result = calculateResizedDimensions(200, -200, -200, 16 / 9, 160, 1920);
  assert.strictEqual(result.width, 160);
  assert.approximately(result.height, 160 / (16 / 9), 0.01);
});

test("calculateResizedDimensions: 最大幅より大きくならない", () => {
  const result = calculateResizedDimensions(1900, 200, 200, 16 / 9, 160, 1920);
  assert.strictEqual(result.width, 1920);
  assert.approximately(result.height, 1920 / (16 / 9), 0.01);
});

test("calculateResizedDimensions: 縮小方向のドラッグでサイズが小さくなる", () => {
  const result = calculateResizedDimensions(400, -60, -60, 16 / 9, 160, 1920);
  assert.strictEqual(result.width, 340);
  assert.approximately(result.height, 340 / (16 / 9), 0.01);
});

// calculateInitialLayout

test("calculateInitialLayout: 1 本のトラックで中央配置する", () => {
  const result = calculateInitialLayout(800, 600, 1, 16 / 9);
  assert.strictEqual(result.length, 1);
  const [item] = result;
  // コンテナ内に収まる
  assert.isAtLeast(item.x, 0);
  assert.isAtLeast(item.y, 0);
  assert.isAtMost(item.x + item.width, 800);
  assert.isAtMost(item.y + item.height, 600);
  // 中央に配置されている
  const centerX = item.x + item.width / 2;
  const centerY = item.y + item.height / 2;
  assert.approximately(centerX, 400, 1);
  assert.approximately(centerY, 300, 1);
});

test("calculateInitialLayout: 2 本のトラックで 2 列配置する", () => {
  const result = calculateInitialLayout(800, 600, 2, 16 / 9);
  assert.strictEqual(result.length, 2);
  // 両方がコンテナ内に収まる
  for (const item of result) {
    assert.isAtLeast(item.x, 0);
    assert.isAtLeast(item.y, 0);
    assert.isAtMost(item.x + item.width, 800);
    assert.isAtMost(item.y + item.height, 600);
  }
  // 左右に分かれている
  assert.isBelow(result[0].x, result[1].x);
});

test("calculateInitialLayout: 4 本のトラックで 2x2 配置する", () => {
  const result = calculateInitialLayout(800, 600, 4, 16 / 9);
  assert.strictEqual(result.length, 4);
  // 全てがコンテナ内に収まる
  for (const item of result) {
    assert.isAtLeast(item.x, 0);
    assert.isAtLeast(item.y, 0);
    assert.isAtMost(item.x + item.width, 800);
    assert.isAtMost(item.y + item.height, 600);
  }
  // 2 行に分かれている
  assert.isBelow(result[0].y, result[2].y);
  // 各行内で左右に分かれている
  assert.isBelow(result[0].x, result[1].x);
  assert.isBelow(result[2].x, result[3].x);
});

test("calculateInitialLayout: アスペクト比が維持される", () => {
  const aspectRatio = 16 / 9;
  const result = calculateInitialLayout(800, 600, 1, aspectRatio);
  const [item] = result;
  assert.approximately(item.width / item.height, aspectRatio, 0.01);
});
