import type { Signal } from "@preact/signals";
import { useState, useEffect, useCallback } from "preact/hooks";
import type { ObsDcConnectionState } from "../../obsdc/client.ts";
import type { RequestResponseData, EventData } from "../../obsdc/protocol.ts";
import { ObsDcModal } from "./ObsDcModal.tsx";

interface ObsDcScenePanelProps {
  connectionState: Signal<ObsDcConnectionState>;
  events: Signal<readonly EventData[]>;
  onSendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

interface Scene {
  readonly sceneName: string;
  readonly sceneIndex: number;
  readonly sceneUuid: string;
}

const SCENE_LIST_REFRESH_EVENT_TYPES = [
  "SceneListChanged",
  "SceneCreated",
  "SceneRemoved",
] as const;

export function ObsDcScenePanel({ connectionState, events, onSendRequest }: ObsDcScenePanelProps) {
  const [scenes, setScenes] = useState<readonly Scene[]>([]);
  const [currentScene, setCurrentScene] = useState("");
  const [loading, setLoading] = useState(false);
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [showRemoveModal, setShowRemoveModal] = useState(false);
  const [newSceneName, setNewSceneName] = useState("");
  const [selectedScene, setSelectedScene] = useState("");

  const isConnected = connectionState.value === "connected";

  const fetchScenes = useCallback(async (): Promise<void> => {
    if (!isConnected) {
      return;
    }
    setLoading(true);
    try {
      const [sceneListResponse, currentResponse] = await Promise.all([
        onSendRequest("GetSceneList"),
        onSendRequest("GetCurrentProgramScene"),
      ]);
      if (sceneListResponse.requestStatus.result && sceneListResponse.responseData) {
        setScenes(sceneListResponse.responseData.scenes as Scene[]);
      }
      if (currentResponse.requestStatus.result && currentResponse.responseData) {
        const name = currentResponse.responseData.sceneName as string;
        setCurrentScene(name);
        // 初回は Program シーンを選択状態にする
        setSelectedScene((prev) => (prev === "" ? name : prev));
      }
    } catch {
      // 失敗時は前回の状態を維持する
    } finally {
      setLoading(false);
    }
  }, [isConnected, onSendRequest]);

  useEffect(() => {
    if (isConnected) {
      void fetchScenes();
      return;
    }
    setScenes([]);
    setCurrentScene("");
    setSelectedScene("");
  }, [isConnected, fetchScenes]);

  // イベントでシーン変更を検知する
  const eventCount = events.value.length;
  useEffect(() => {
    if (eventCount === 0) {
      return;
    }
    const latestEvent = events.value.at(-1);
    if (latestEvent === undefined) {
      return;
    }
    if (latestEvent.eventType === "CurrentProgramSceneChanged" && latestEvent.eventData) {
      setCurrentScene(latestEvent.eventData.sceneName as string);
    }
    if (
      SCENE_LIST_REFRESH_EVENT_TYPES.includes(
        latestEvent.eventType as (typeof SCENE_LIST_REFRESH_EVENT_TYPES)[number],
      )
    ) {
      void fetchScenes();
    }
  }, [eventCount, events.value, fetchScenes]);

  async function handleSceneSwitch(sceneName: string): Promise<void> {
    try {
      await onSendRequest("SetCurrentProgramScene", { sceneName });
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  async function handleCreateScene(): Promise<void> {
    if (newSceneName.trim() === "") {
      return;
    }
    try {
      await onSendRequest("CreateScene", { sceneName: newSceneName.trim() });
      setShowCreateModal(false);
      setNewSceneName("");
      await fetchScenes();
    } catch {
      // 失敗時はモーダルを開いたままにする
    }
  }

  async function handleRemoveScene(): Promise<void> {
    if (selectedScene === "") {
      return;
    }
    try {
      await onSendRequest("RemoveScene", { sceneName: selectedScene });
      setShowRemoveModal(false);
      setSelectedScene("");
      await fetchScenes();
    } catch {
      // 失敗時はモーダルを開いたままにする
    }
  }

  // 現在の Program シーンは削除不可
  const canRemove = selectedScene !== "" && selectedScene !== currentScene;

  if (!isConnected) {
    return (
      <div class="flex h-full items-center justify-center text-sm text-slate-500">
        Not connected
      </div>
    );
  }

  return (
    <div class="flex h-full flex-col">
      {/* シーンリスト */}
      <div class="flex-1 overflow-y-auto">
        {scenes.length === 0 ? (
          <div class="flex h-full items-center justify-center text-sm text-slate-500">
            {loading ? "Loading..." : "No scenes"}
          </div>
        ) : (
          <div class="flex flex-col">
            {[...scenes].toReversed().map((scene) => {
              const isCurrent = scene.sceneName === currentScene;
              const isSelected = scene.sceneName === selectedScene;
              return (
                <div
                  key={scene.sceneUuid}
                  class={`flex items-center border-b border-surface-200/50 px-3 py-2 ${
                    isSelected ? "bg-accent-50 ring-1 ring-inset ring-accent-200" : "hover:bg-surface-100/60"
                  }`}
                >
                  {/* 選択 */}
                  <button
                    type="button"
                    onClick={() => {
                      setSelectedScene(scene.sceneName);
                    }}
                    class="flex flex-1 items-center gap-2 text-left"
                  >
                    {/* Program インジケーター */}
                    <span
                      class={`inline-block h-2 w-2 shrink-0 rounded-full ${
                        isCurrent
                          ? "bg-red-500 shadow-[0_0_6px_rgba(239,68,68,0.5)]"
                          : "bg-transparent"
                      }`}
                    />
                    <span
                      class={`text-sm ${isCurrent ? "font-semibold text-slate-900" : "text-slate-600"}`}
                    >
                      {scene.sceneName}
                    </span>
                  </button>
                  {/* Program 切り替えボタン */}
                  {!isCurrent && (
                    <button
                      type="button"
                      onClick={() => {
                        void handleSceneSwitch(scene.sceneName);
                      }}
                      class="shrink-0 rounded px-2 py-0.5 text-xs text-slate-500 hover:bg-surface-300 hover:text-slate-800"
                      title="Program に切り替え"
                    >
                      Switch
                    </button>
                  )}
                  {isCurrent && (
                    <span class="shrink-0 rounded bg-red-50 px-2 py-0.5 text-xs font-medium text-red-700">
                      PGM
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* ツールバー */}
      <div class="flex items-center gap-1 border-t border-surface-200 px-2 py-1.5">
        <button
          type="button"
          onClick={() => {
            setNewSceneName("");
            setShowCreateModal(true);
          }}
          disabled={!isConnected}
          class="rounded px-2 py-1 text-sm text-slate-600 hover:bg-surface-200 hover:text-slate-900 disabled:opacity-50"
          title="シーンを追加"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <line x1="8" y1="3" x2="8" y2="13" />
            <line x1="3" y1="8" x2="13" y2="8" />
          </svg>
        </button>
        <button
          type="button"
          onClick={() => {
            setShowRemoveModal(true);
          }}
          disabled={!canRemove}
          class="rounded px-2 py-1 text-sm text-slate-600 hover:bg-surface-200 hover:text-red-700 disabled:opacity-30"
          title="シーンを削除"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <line x1="3" y1="8" x2="13" y2="8" />
          </svg>
        </button>
        <div class="mx-1 h-4 w-px bg-surface-200" />
        <button
          type="button"
          onClick={() => {
            void fetchScenes();
          }}
          disabled={loading}
          class="rounded px-2 py-1 text-slate-600 hover:bg-surface-200 hover:text-slate-900 disabled:opacity-50"
          title="再取得"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M13 8a5 5 0 0 1-8.54 3.54M3 8a5 5 0 0 1 8.54-3.54" />
            <polyline points="13 3 13 8 8 8" />
            <polyline points="3 13 3 8 8 8" />
          </svg>
        </button>
      </div>

      {/* シーン作成モーダル */}
      <ObsDcModal
        open={showCreateModal}
        title="Add Scene"
        onClose={() => {
          setShowCreateModal(false);
        }}
      >
        <div class="flex flex-col gap-3">
          <div class="flex flex-col gap-1">
            <label class="text-sm text-slate-600">Scene Name</label>
            <input
              type="text"
              value={newSceneName}
              onInput={(event) => {
                setNewSceneName((event.target as HTMLInputElement).value);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  void handleCreateScene();
                }
              }}
              placeholder="Scene Name"
              class="rounded border border-surface-200 bg-white px-2 py-1.5 text-sm text-slate-900 placeholder:text-slate-400"
            />
          </div>
          <div class="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => {
                setShowCreateModal(false);
              }}
              class="rounded bg-surface-200 px-3 py-1 text-sm text-slate-600 hover:bg-surface-300"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => {
                void handleCreateScene();
              }}
              disabled={newSceneName.trim() === ""}
              class="rounded bg-accent-600 px-3 py-1 text-sm text-white hover:bg-accent-500 disabled:opacity-50"
            >
              Add
            </button>
          </div>
        </div>
      </ObsDcModal>

      {/* シーン削除確認モーダル */}
      <ObsDcModal
        open={showRemoveModal}
        title="Remove Scene"
        onClose={() => {
          setShowRemoveModal(false);
        }}
      >
        <div class="flex flex-col gap-3">
          <p class="text-sm text-slate-600">
            Remove scene <span class="font-medium text-slate-900">&quot;{selectedScene}&quot;</span>
            ?
          </p>
          <div class="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => {
                setShowRemoveModal(false);
              }}
              class="rounded bg-surface-200 px-3 py-1 text-sm text-slate-600 hover:bg-surface-300"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => {
                void handleRemoveScene();
              }}
              class="rounded bg-red-800/80 px-3 py-1 text-sm text-red-100 hover:bg-red-700/80"
            >
              Delete
            </button>
          </div>
        </div>
      </ObsDcModal>
    </div>
  );
}
