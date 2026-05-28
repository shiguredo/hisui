import type { Signal } from "@preact/signals";
import type { RefObject } from "preact";
import { useRef, useEffect, useState, useCallback } from "preact/hooks";

import {
  snapPosition,
  clampPosition,
  calculateResizedDimensions,
  calculateInitialLayout,
} from "./videoDisplayLayout.ts";
import type { Rect } from "./videoDisplayLayout.ts";

interface VideoDisplayProps {
  tracks: Signal<readonly MediaStreamTrack[]>;
}

interface CanvasElementProps {
  track: MediaStreamTrack;
  trackIndex: number;
  rect: Rect;
  onDragStart: (index: number, clientX: number, clientY: number) => void;
  onResizeStart: (index: number, clientX: number, clientY: number) => void;
  isDragging: boolean;
  containerRef: RefObject<HTMLDivElement>;
  onInitialFrame: (index: number, aspectRatio: number) => void;
  trackCount: number;
}

const MIN_WIDTH = 160;
const MAX_WIDTH = 1920;

function CanvasElement({
  track,
  trackIndex,
  rect,
  onDragStart,
  onResizeStart,
  isDragging,
  containerRef,
  onInitialFrame,
  trackCount,
}: CanvasElementProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const layoutInitializedRef = useRef(false);

  useEffect(() => {
    if (canvasRef.current === null) {
      return;
    }
    const canvas = canvasRef.current;
    const context = canvas.getContext("2d");
    if (context === null) {
      return;
    }

    const processor = new MediaStreamTrackProcessor({ track });
    const reader = processor.readable.getReader();
    let cancelled = false;

    const renderLoop = async (): Promise<void> => {
      for (;;) {
        if (cancelled) {
          break;
        }
        const { done, value: frame } = await reader.read();
        if (done) {
          break;
        }

        canvas.width = frame.displayWidth;
        canvas.height = frame.displayHeight;
        context.drawImage(frame, 0, 0);

        if (!layoutInitializedRef.current) {
          const ratio = frame.displayWidth / frame.displayHeight;
          onInitialFrame(trackIndex, ratio);
          layoutInitializedRef.current = true;
        }

        frame.close();
      }
    };
    void renderLoop();

    return () => {
      cancelled = true;
      layoutInitializedRef.current = false;
      void reader.cancel();
    };
  }, [track, trackIndex, trackCount, containerRef, onInitialFrame]);

  const handleDragPointerDown = useCallback(
    (event: PointerEvent) => {
      const target = event.currentTarget;
      if (!(target instanceof HTMLElement)) {
        return;
      }
      target.setPointerCapture(event.pointerId);
      onDragStart(trackIndex, event.clientX, event.clientY);
    },
    [trackIndex, onDragStart],
  );

  const handleResizePointerDown = useCallback(
    (event: PointerEvent) => {
      event.stopPropagation();
      const target = event.currentTarget;
      if (!(target instanceof HTMLElement)) {
        return;
      }
      target.setPointerCapture(event.pointerId);
      onResizeStart(trackIndex, event.clientX, event.clientY);
    },
    [trackIndex, onResizeStart],
  );

  if (rect.width === 0 && rect.height === 0) {
    return (
      <div class="absolute">
        <canvas ref={canvasRef} class="hidden" />
      </div>
    );
  }

  return (
    <div
      class="absolute overflow-hidden rounded-md border border-surface-300 bg-black shadow-sm"
      style={{
        left: `${rect.x}px`,
        top: `${rect.y}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
        cursor: isDragging ? "grabbing" : "grab",
      }}
      onPointerDown={handleDragPointerDown}
    >
      <canvas
        ref={canvasRef}
        class="h-full w-full object-contain"
        style={{ pointerEvents: "none" }}
      />
      {/* リサイズハンドル */}
      <div
        class="absolute right-0 bottom-0 flex h-4 w-4 items-center justify-center"
        style={{ cursor: "nwse-resize" }}
        onPointerDown={handleResizePointerDown}
      >
        <svg width="10" height="10" viewBox="0 0 10 10">
          <polygon points="10,0 10,10 0,10" fill="rgba(255,255,255,0.5)" />
        </svg>
      </div>
    </div>
  );
}

export function VideoDisplay({ tracks }: VideoDisplayProps) {
  const videoTracks = tracks.value.filter((track) => track.kind === "video");
  const videoTrackCount = videoTracks.length;
  const containerRef = useRef<HTMLDivElement>(null);

  // 全ソースの位置・サイズを一元管理する
  const [rects, setRects] = useState<Rect[]>([]);
  const aspectRatiosRef = useRef<Map<number, number>>(new Map());

  // ドラッグ状態
  const [draggingIndex, setDraggingIndex] = useState<number | null>(null);
  const dragStartRef = useRef<{ clientX: number; clientY: number; rect: Rect }>({
    clientX: 0,
    clientY: 0,
    rect: { x: 0, y: 0, width: 0, height: 0 },
  });

  // リサイズ状態
  const [resizingIndex, setResizingIndex] = useState<number | null>(null);
  const resizeStartRef = useRef<{ clientX: number; clientY: number; rect: Rect }>({
    clientX: 0,
    clientY: 0,
    rect: { x: 0, y: 0, width: 0, height: 0 },
  });

  // トラック数が変わったら rects をリセットする
  useEffect(() => {
    setRects(Array.from({ length: videoTrackCount }, () => ({ x: 0, y: 0, width: 0, height: 0 })));
    aspectRatiosRef.current.clear();
  }, [videoTrackCount]);

  // 初回フレーム受信時に初期レイアウトを計算する
  const handleInitialFrame = useCallback(
    (index: number, aspectRatio: number) => {
      aspectRatiosRef.current.set(index, aspectRatio);

      if (containerRef.current === null) {
        return;
      }
      const container = containerRef.current;
      const layout = calculateInitialLayout(
        container.clientWidth,
        container.clientHeight,
        videoTrackCount,
        aspectRatio,
      );

      setRects((prev) => {
        const next = [...prev];
        next[index] = layout[index];
        return next;
      });
    },
    [videoTrackCount],
  );

  // ドラッグ開始
  const handleDragStart = useCallback(
    (index: number, clientX: number, clientY: number) => {
      setDraggingIndex(index);
      dragStartRef.current = {
        clientX,
        clientY,
        rect: { ...rects[index] },
      };
    },
    [rects],
  );

  // リサイズ開始
  const handleResizeStart = useCallback(
    (index: number, clientX: number, clientY: number) => {
      setResizingIndex(index);
      resizeStartRef.current = {
        clientX,
        clientY,
        rect: { ...rects[index] },
      };
    },
    [rects],
  );

  // コンテナ上でのポインタ移動
  const handlePointerMove = useCallback(
    (event: PointerEvent) => {
      if (containerRef.current === null) {
        return;
      }
      const container = containerRef.current;
      const containerWidth = container.clientWidth;
      const containerHeight = container.clientHeight;

      if (draggingIndex !== null) {
        const deltaX = event.clientX - dragStartRef.current.clientX;
        const deltaY = event.clientY - dragStartRef.current.clientY;
        const startRect = dragStartRef.current.rect;
        const rawX = startRect.x + deltaX;
        const rawY = startRect.y + deltaY;

        // スナップ対象: 自分以外の全矩形
        const others = rects.filter((_, i) => i !== draggingIndex);
        const candidate: Rect = {
          x: rawX,
          y: rawY,
          width: startRect.width,
          height: startRect.height,
        };
        const snapped = snapPosition(candidate, others, containerWidth, containerHeight);
        const clamped = clampPosition(
          snapped.x,
          snapped.y,
          startRect.width,
          startRect.height,
          containerWidth,
          containerHeight,
        );

        setRects((prev) => {
          const next = [...prev];
          next[draggingIndex] = { ...prev[draggingIndex], x: clamped.x, y: clamped.y };
          return next;
        });
      }

      if (resizingIndex !== null) {
        const deltaX = event.clientX - resizeStartRef.current.clientX;
        const deltaY = event.clientY - resizeStartRef.current.clientY;
        const startRect = resizeStartRef.current.rect;
        const aspectRatio = aspectRatiosRef.current.get(resizingIndex) ?? 16 / 9;

        const resized = calculateResizedDimensions(
          startRect.width,
          deltaX,
          deltaY,
          aspectRatio,
          MIN_WIDTH,
          Math.min(MAX_WIDTH, containerWidth),
        );

        const clamped = clampPosition(
          startRect.x,
          startRect.y,
          resized.width,
          resized.height,
          containerWidth,
          containerHeight,
        );

        setRects((prev) => {
          const next = [...prev];
          next[resizingIndex] = {
            x: clamped.x,
            y: clamped.y,
            width: resized.width,
            height: resized.height,
          };
          return next;
        });
      }
    },
    [draggingIndex, resizingIndex, rects],
  );

  // ポインタ離し
  const handlePointerUp = useCallback(() => {
    setDraggingIndex(null);
    setResizingIndex(null);
  }, []);

  if (videoTracks.length === 0) {
    return (
      <div class="flex h-full items-center justify-center text-lg text-slate-500">No video</div>
    );
  }

  return (
    <div
      ref={containerRef}
      class="relative h-full w-full overflow-hidden"
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      {videoTracks.map((track, index) => (
        <CanvasElement
          key={track.id}
          track={track}
          trackIndex={index}
          rect={rects[index] ?? { x: 0, y: 0, width: 0, height: 0 }}
          onDragStart={handleDragStart}
          onResizeStart={handleResizeStart}
          isDragging={draggingIndex === index}
          containerRef={containerRef}
          onInitialFrame={handleInitialFrame}
          trackCount={videoTrackCount}
        />
      ))}
    </div>
  );
}
