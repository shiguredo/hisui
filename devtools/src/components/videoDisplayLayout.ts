export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

// スナップ閾値 (px)
const SNAP_THRESHOLD = 8;

// 他の矩形の辺やコンテナの端・中央にスナップした位置を返す
export function snapPosition(
  rect: Rect,
  others: readonly Rect[],
  containerWidth: number,
  containerHeight: number,
): { x: number; y: number } {
  let snappedX = rect.x;
  let snappedY = rect.y;
  let bestDx = SNAP_THRESHOLD + 1;
  let bestDy = SNAP_THRESHOLD + 1;

  // 自分の辺
  const left = rect.x;
  const right = rect.x + rect.width;
  const centerX = rect.x + rect.width / 2;
  const top = rect.y;
  const bottom = rect.y + rect.height;
  const centerY = rect.y + rect.height / 2;

  // スナップ候補の X 座標ペア: [自分の辺の値, スナップ先の値]
  const xCandidates: Array<[number, number]> = [
    // コンテナの左端
    [left, 0],
    // コンテナの右端
    [right, containerWidth],
    // コンテナの中央
    [centerX, containerWidth / 2],
  ];

  const yCandidates: Array<[number, number]> = [
    // コンテナの上端
    [top, 0],
    // コンテナの下端
    [bottom, containerHeight],
    // コンテナの中央
    [centerY, containerHeight / 2],
  ];

  // 他の矩形の辺をスナップ候補に追加する
  for (const other of others) {
    const otherLeft = other.x;
    const otherRight = other.x + other.width;
    const otherCenterX = other.x + other.width / 2;
    const otherTop = other.y;
    const otherBottom = other.y + other.height;
    const otherCenterY = other.y + other.height / 2;

    // X 方向: 自分の左辺 → 相手の左辺・右辺、自分の右辺 → 相手の左辺・右辺、中央同士
    xCandidates.push([left, otherLeft], [left, otherRight]);
    xCandidates.push([right, otherLeft], [right, otherRight]);
    xCandidates.push([centerX, otherCenterX]);

    // Y 方向: 同様
    yCandidates.push([top, otherTop], [top, otherBottom]);
    yCandidates.push([bottom, otherTop], [bottom, otherBottom]);
    yCandidates.push([centerY, otherCenterY]);
  }

  for (const [selfEdge, targetEdge] of xCandidates) {
    const distance = Math.abs(selfEdge - targetEdge);
    if (distance < bestDx) {
      bestDx = distance;
      snappedX = rect.x + (targetEdge - selfEdge);
    }
  }

  for (const [selfEdge, targetEdge] of yCandidates) {
    const distance = Math.abs(selfEdge - targetEdge);
    if (distance < bestDy) {
      bestDy = distance;
      snappedY = rect.y + (targetEdge - selfEdge);
    }
  }

  // 閾値を超えた場合は元の位置を使う
  if (bestDx > SNAP_THRESHOLD) {
    snappedX = rect.x;
  }
  if (bestDy > SNAP_THRESHOLD) {
    snappedY = rect.y;
  }

  return { x: snappedX, y: snappedY };
}

// 位置をコンテナ内にクランプする
export function clampPosition(
  x: number,
  y: number,
  elementWidth: number,
  elementHeight: number,
  containerWidth: number,
  containerHeight: number,
): { x: number; y: number } {
  const clampedX = Math.max(0, Math.min(x, containerWidth - elementWidth));
  const clampedY = Math.max(0, Math.min(y, containerHeight - elementHeight));
  return { x: clampedX, y: clampedY };
}

// アスペクト比を維持したリサイズ後のサイズを計算する
export function calculateResizedDimensions(
  startWidth: number,
  deltaX: number,
  deltaY: number,
  aspectRatio: number,
  minWidth: number,
  maxWidth: number,
): { width: number; height: number } {
  const delta = (deltaX + deltaY) / 2;
  const newWidth = Math.max(minWidth, Math.min(startWidth + delta, maxWidth));
  const newHeight = newWidth / aspectRatio;
  return { width: newWidth, height: newHeight };
}

interface LayoutItem {
  x: number;
  y: number;
  width: number;
  height: number;
}

// トラック数に応じた初期配置を計算する
export function calculateInitialLayout(
  containerWidth: number,
  containerHeight: number,
  trackCount: number,
  aspectRatio: number,
): LayoutItem[] {
  if (trackCount === 0) {
    return [];
  }

  const columns = trackCount === 1 ? 1 : 2;
  const rows = Math.ceil(trackCount / columns);

  // セルのサイズを計算する
  const cellWidth = containerWidth / columns;
  const cellHeight = containerHeight / rows;

  // セル内でアスペクト比を維持した最大サイズを計算する
  const cellAspectRatio = cellWidth / cellHeight;
  let itemWidth: number;
  let itemHeight: number;

  if (cellAspectRatio > aspectRatio) {
    // セルが横長なので高さに合わせる
    itemHeight = cellHeight;
    itemWidth = cellHeight * aspectRatio;
  } else {
    // セルが縦長なので幅に合わせる
    itemWidth = cellWidth;
    itemHeight = cellWidth / aspectRatio;
  }

  const items: LayoutItem[] = [];

  for (let index = 0; index < trackCount; index++) {
    const column = index % columns;
    const row = Math.floor(index / columns);

    // セルの左上座標
    const cellX = column * cellWidth;
    const cellY = row * cellHeight;

    // セル内でセンタリング
    const x = cellX + (cellWidth - itemWidth) / 2;
    const y = cellY + (cellHeight - itemHeight) / 2;

    items.push({ x, y, width: itemWidth, height: itemHeight });
  }

  return items;
}
