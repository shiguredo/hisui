import type { Signal } from "@preact/signals";
import { useState, useEffect, useCallback } from "preact/hooks";
import type { ObsDcConnectionState } from "../../obsdc/client.ts";
import type { RequestResponseData, EventData } from "../../obsdc/protocol.ts";
import { ObsDcModal } from "./ObsDcModal.tsx";

interface ObsDcSourcePanelProps {
  connectionState: Signal<ObsDcConnectionState>;
  events: Signal<readonly EventData[]>;
  onSendRequest: (
    requestType: string,
    requestData?: Record<string, unknown>,
  ) => Promise<RequestResponseData>;
}

interface SceneItem {
  readonly sceneItemId: number;
  readonly sceneItemIndex: number;
  readonly sceneItemEnabled: boolean;
  readonly sourceName: string;
  readonly sourceType: string;
  readonly sourceUuid: string;
  readonly inputKind: string | null;
  readonly isGroup: boolean | null;
}

interface InputSettingsFieldProps {
  label: string;
  settingsKey: string;
  placeholder: string;
  inputSettings: Record<string, string>;
  setInputSettings: (settings: Record<string, string>) => void;
}

function InputSettingsField({
  label,
  settingsKey,
  placeholder,
  inputSettings,
  setInputSettings,
}: InputSettingsFieldProps) {
  return (
    <div class="flex flex-col gap-1">
      <label class="text-sm text-slate-600">{label}</label>
      <input
        type="text"
        value={inputSettings[settingsKey] ?? ""}
        onInput={(event) => {
          setInputSettings({
            ...inputSettings,
            [settingsKey]: (event.target as HTMLInputElement).value,
          });
        }}
        placeholder={placeholder}
        class="rounded border border-surface-200 bg-white px-2 py-1.5 text-sm text-slate-900 placeholder:text-slate-400"
      />
    </div>
  );
}

interface PropertyItem {
  readonly itemName: string;
  readonly itemValue: string;
  readonly itemEnabled: boolean;
}

interface DeviceSelectFieldProps {
  label: string;
  settingsKey: string;
  inputSettings: Record<string, string>;
  setInputSettings: (settings: Record<string, string>) => void;
  devices: readonly PropertyItem[];
  loading: boolean;
}

// デバイス選択ドロップダウン
function DeviceSelectField({
  label,
  settingsKey,
  inputSettings,
  setInputSettings,
  devices,
  loading,
}: DeviceSelectFieldProps) {
  return (
    <div class="flex flex-col gap-1">
      <label class="text-sm text-slate-600">{label}</label>
      {loading ? (
        <div class="text-sm text-slate-600">Loading...</div>
      ) : (
        <select
          value={inputSettings[settingsKey] ?? ""}
          onChange={(event) => {
            setInputSettings({
              ...inputSettings,
              [settingsKey]: (event.target as HTMLSelectElement).value,
            });
          }}
          class="rounded border border-surface-200 bg-white px-2 py-1.5 text-sm text-slate-900"
        >
          <option value="">---</option>
          {devices.map((device) => (
            <option key={device.itemValue} value={device.itemValue}>
              {device.itemName}
            </option>
          ))}
        </select>
      )}
    </div>
  );
}

// 表示/非表示アイコン
function EyeIcon({ visible }: { visible: boolean }) {
  if (visible) {
    return (
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
        <path d="M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8s-2.5 4.5-6.5 4.5S1.5 8 1.5 8z" />
        <circle cx="8" cy="8" r="2" />
      </svg>
    );
  }
  return (
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
      <path d="M6.3 6.3a2 2 0 0 0 3.4 3.4" />
      <path d="M10.6 10.7C9.8 11.3 8.9 11.5 8 11.5c-4 0-6.5-3.5-6.5-3.5a10 10 0 0 1 3.1-2.7" />
      <path d="M5.7 3.7A5.5 5.5 0 0 1 8 3.5c4 0 6.5 4.5 6.5 4.5a10 10 0 0 1-1.3 1.7" />
      <line x1="2" y1="2" x2="14" y2="14" />
    </svg>
  );
}

const INPUT_KIND_LABELS: Record<string, string> = {
  video_capture_device: "Video Capture Device",
  audio_capture_device: "Audio Capture Device",
  image_source: "Image",
  mp4_file_source: "Media Source",
  browser_source: "Browser",
  text_ft2_source_v2: "Text",
  color_source_v3: "Color Source",
  rtmp_inbound: "RTMP Source",
  srt_inbound: "SRT Source",
  rtsp_subscriber: "RTSP Source",
  window_capture: "Window Capture",
  monitor_capture: "Display Capture",
  game_capture: "Game Capture",
  pipewire_desktop_capture_source: "PipeWire Capture",
};

// ソースタイプのラベル
function sourceKindLabel(inputKind: string | null): string {
  if (inputKind === null) {
    return "";
  }
  return INPUT_KIND_LABELS[inputKind] ?? inputKind;
}

export function ObsDcSourcePanel({
  connectionState,
  events,
  onSendRequest,
}: ObsDcSourcePanelProps) {
  const [sceneItems, setSceneItems] = useState<readonly SceneItem[]>([]);
  const [currentScene, setCurrentScene] = useState("");
  const [loading, setLoading] = useState(false);
  const [selectedItemId, setSelectedItemId] = useState<number | null>(null);
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [showRemoveModal, setShowRemoveModal] = useState(false);
  const [newInputName, setNewInputName] = useState("");
  const [newInputKind, setNewInputKind] = useState("");
  const [inputKinds, setInputKinds] = useState<readonly string[]>([]);
  const [loadingKinds, setLoadingKinds] = useState(false);
  const [inputSettings, setInputSettings] = useState<Record<string, string>>({});
  const [deviceItems, setDeviceItems] = useState<readonly PropertyItem[]>([]);
  const [loadingDevices, setLoadingDevices] = useState(false);

  const isConnected = connectionState.value === "connected";

  const fetchItems = useCallback(async (): Promise<void> => {
    if (!isConnected) {
      return;
    }
    setLoading(true);
    try {
      const sceneResponse = await onSendRequest("GetCurrentProgramScene");
      if (!sceneResponse.requestStatus.result || !sceneResponse.responseData) {
        return;
      }
      const sceneName = sceneResponse.responseData.sceneName as string;
      setCurrentScene(sceneName);
      const itemListResponse = await onSendRequest("GetSceneItemList", { sceneName });
      if (itemListResponse.requestStatus.result && itemListResponse.responseData) {
        setSceneItems(itemListResponse.responseData.sceneItems as SceneItem[]);
      }
    } catch {
      // 失敗時は前回の状態を維持する
    } finally {
      setLoading(false);
    }
  }, [isConnected, onSendRequest]);

  useEffect(() => {
    if (isConnected) {
      void fetchItems();
      return;
    }
    setSceneItems([]);
    setCurrentScene("");
    setSelectedItemId(null);
  }, [isConnected, fetchItems]);

  // シーン変更時にリフレッシュする
  const eventCount = events.value.length;
  useEffect(() => {
    if (eventCount === 0) {
      return;
    }
    const latestEvent = events.value.at(-1);
    if (latestEvent === undefined) {
      return;
    }
    if (latestEvent.eventType === "CurrentProgramSceneChanged") {
      setSelectedItemId(null);
      void fetchItems();
    }
    if (
      latestEvent.eventType === "SceneItemCreated" ||
      latestEvent.eventType === "SceneItemRemoved" ||
      latestEvent.eventType === "SceneItemEnableStateChanged"
    ) {
      void fetchItems();
    }
  }, [eventCount, events.value, fetchItems]);

  async function toggleItemEnabled(sceneItemId: number, currentEnabled: boolean): Promise<void> {
    try {
      await onSendRequest("SetSceneItemEnabled", {
        sceneName: currentScene,
        sceneItemId,
        sceneItemEnabled: !currentEnabled,
      });
      await fetchItems();
    } catch {
      // 失敗時はイベントまたは手動 Refresh で状態を同期する
    }
  }

  const fetchInputKinds = useCallback(async (): Promise<void> => {
    setLoadingKinds(true);
    try {
      const response = await onSendRequest("GetInputKindList", { unversioned: true });
      if (response.requestStatus.result && response.responseData) {
        const kinds = response.responseData.inputKinds as string[];
        setInputKinds(kinds.toSorted());
      }
    } catch {
      // 失敗時は前回の種類一覧を維持する
    } finally {
      setLoadingKinds(false);
    }
  }, [onSendRequest]);

  function handleOpenCreateModal(): void {
    setNewInputName("");
    setNewInputKind("");
    setInputSettings({});
    setDeviceItems([]);
    setShowCreateModal(true);
    void fetchInputKinds();
  }

  // デバイス列挙用の一時 input 名（モーダルを閉じる際に削除する）
  const [probeInputName, setProbeInputName] = useState<string | null>(null);

  // 一時 input を削除する
  async function cleanupProbeInput(): Promise<void> {
    if (probeInputName === null) {
      return;
    }
    const nameToRemove = probeInputName;
    setProbeInputName(null);
    try {
      const response = await onSendRequest("GetSceneItemId", {
        sceneName: currentScene,
        sourceName: nameToRemove,
      });
      if (response.requestStatus.result && response.responseData) {
        const sceneItemId = response.responseData.sceneItemId as number;
        await onSendRequest("RemoveSceneItem", {
          sceneName: currentScene,
          sceneItemId,
        });
      }
      await onSendRequest("RemoveInput", { inputName: nameToRemove });
    } catch {
      // 一時 input の削除失敗は UI 操作を妨げない
    }
  }

  const fetchDeviceList = useCallback(
    async (inputKind: string): Promise<void> => {
      setLoadingDevices(true);
      setDeviceItems([]);
      try {
        // 指定した inputKind の既存 input を探し、なければ一時 input を作成する
        let inputName: string | undefined;
        const listResponse = await onSendRequest("GetInputList", { inputKind });
        if (listResponse.requestStatus.result && listResponse.responseData) {
          const inputs = listResponse.responseData.inputs as ReadonlyArray<{
            inputName: string;
          }>;
          if (inputs.length > 0) {
            const [{ inputName: existingInputName }] = inputs;
            inputName = existingInputName;
          }
        }
        if (inputName === undefined) {
          const tempName = `__probe_${inputKind}_${Date.now()}`;
          const createResponse = await onSendRequest("CreateInput", {
            inputName: tempName,
            inputKind,
            sceneName: currentScene,
            inputSettings: {},
            sceneItemEnabled: false,
          });
          if (createResponse.requestStatus.result) {
            setProbeInputName(tempName);
            inputName = tempName;
          }
        }
        if (inputName === undefined) {
          return;
        }
        const response = await onSendRequest("GetInputPropertiesListPropertyItems", {
          inputName,
          propertyName: "device_id",
        });
        if (response.requestStatus.result && response.responseData) {
          setDeviceItems(response.responseData.propertyItems as PropertyItem[]);
        }
      } catch {
        // 失敗時は空のデバイス一覧を維持する
      } finally {
        setLoadingDevices(false);
      }
    },
    [currentScene, onSendRequest],
  );

  function generateDefaultInputName(kind: string): string {
    const baseName = INPUT_KIND_LABELS[kind] ?? kind;
    const existingNames = new Set(sceneItems.map((item) => item.sourceName));
    if (!existingNames.has(baseName)) {
      return baseName;
    }
    let suffix = 2;
    while (existingNames.has(`${baseName} ${suffix}`)) {
      suffix++;
    }
    return `${baseName} ${suffix}`;
  }

  function handleInputKindChange(kind: string): void {
    setNewInputKind(kind);
    setInputSettings({});
    setDeviceItems([]);
    setNewInputName(kind !== "" ? generateDefaultInputName(kind) : "");
    if (kind === "video_capture_device" || kind === "audio_capture_device") {
      void fetchDeviceList(kind);
    }
  }

  function handleDeviceChange(deviceId: string): void {
    setInputSettings((prev) => ({ ...prev, device_id: deviceId }));
  }

  function buildInputSettings(): Record<string, unknown> {
    const settings: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(inputSettings)) {
      if (value.trim() === "") {
        continue;
      }
      settings[key] = value.trim();
    }
    return settings;
  }

  async function handleCreateInput(): Promise<void> {
    if (newInputName.trim() === "" || newInputKind === "" || currentScene === "") {
      return;
    }
    // 一時 input がある場合は先に削除する
    await cleanupProbeInput();
    try {
      await onSendRequest("CreateInput", {
        inputName: newInputName.trim(),
        inputKind: newInputKind,
        sceneName: currentScene,
        inputSettings: buildInputSettings(),
      });
      setShowCreateModal(false);
      setNewInputName("");
      setNewInputKind("");
      setInputSettings({});
      await fetchItems();
    } catch {
      // 失敗時はモーダルを開いたままにする
    }
  }

  async function handleRemoveItem(): Promise<void> {
    if (selectedItemId === null || currentScene === "") {
      return;
    }
    const inputName = selectedItem?.sourceName;
    if (inputName === undefined) {
      return;
    }
    try {
      await onSendRequest("RemoveInput", { inputName });
      setShowRemoveModal(false);
      setSelectedItemId(null);
      await fetchItems();
    } catch {
      // 失敗時はモーダルを開いたままにする
    }
  }

  const selectedItem = sceneItems.find((item) => item.sceneItemId === selectedItemId);

  if (!isConnected) {
    return (
      <div class="flex h-full items-center justify-center text-sm text-slate-500">
        Not connected
      </div>
    );
  }

  return (
    <div class="flex h-full flex-col">
      {/* シーン名ヘッダー */}
      {currentScene !== "" && (
        <div class="border-b border-surface-200 px-3 py-1.5 text-xs text-slate-500">
          Scene: <span class="text-slate-600">{currentScene}</span>
        </div>
      )}

      {/* ソースリスト */}
      <div class="flex-1 overflow-y-auto">
        {sceneItems.length === 0 ? (
          <div class="flex h-full items-center justify-center text-sm text-slate-500">
            {loading ? "Loading..." : "No sources"}
          </div>
        ) : (
          <div class="flex flex-col">
            {[...sceneItems].toReversed().map((item) => {
              const isSelected = item.sceneItemId === selectedItemId;
              const kindLabel = sourceKindLabel(item.inputKind);
              return (
                <div
                  key={item.sceneItemId}
                  class={`flex items-center gap-2 border-b border-surface-200/50 px-3 py-2 ${
                    isSelected ? "bg-accent-50 ring-1 ring-inset ring-accent-200" : "hover:bg-surface-100/60"
                  }`}
                >
                  {/* 表示/非表示トグル */}
                  <button
                    type="button"
                    onClick={() => {
                      void toggleItemEnabled(item.sceneItemId, item.sceneItemEnabled);
                    }}
                    class={`shrink-0 rounded p-0.5 ${
                      item.sceneItemEnabled
                        ? "text-slate-600 hover:text-slate-900"
                        : "text-slate-600 hover:text-slate-500"
                    }`}
                    title={item.sceneItemEnabled ? "表示中" : "非表示"}
                  >
                    <EyeIcon visible={item.sceneItemEnabled} />
                  </button>
                  {/* ソース名 */}
                  <button
                    type="button"
                    onClick={() => {
                      setSelectedItemId(item.sceneItemId);
                    }}
                    class="flex flex-1 items-center gap-2 text-left"
                  >
                    <span
                      class={`text-sm ${item.sceneItemEnabled ? "text-slate-800" : "text-slate-500"}`}
                    >
                      {item.sourceName}
                    </span>
                    {kindLabel !== "" && (
                      <span class="rounded bg-surface-100 px-1.5 py-0.5 text-[11px] text-slate-500">
                        {kindLabel}
                      </span>
                    )}
                  </button>
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
          onClick={handleOpenCreateModal}
          disabled={currentScene === ""}
          class="rounded px-2 py-1 text-sm text-slate-600 hover:bg-surface-200 hover:text-slate-900 disabled:opacity-50"
          title="ソースを追加"
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
          disabled={selectedItemId === null}
          class="rounded px-2 py-1 text-sm text-slate-600 hover:bg-surface-200 hover:text-red-700 disabled:opacity-30"
          title="ソースを削除"
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
            void fetchItems();
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

      {/* ソース追加モーダル */}
      <ObsDcModal
        open={showCreateModal}
        title="Add Source"
        onClose={() => {
          void cleanupProbeInput();
          setShowCreateModal(false);
        }}
      >
        <div class="flex flex-col gap-3">
          <div class="flex flex-col gap-1">
            <label class="text-sm text-slate-600">Input Kind</label>
            {loadingKinds ? (
              <div class="text-sm text-slate-600">Loading...</div>
            ) : (
              <select
                value={newInputKind}
                onChange={(event) => {
                  handleInputKindChange((event.target as HTMLSelectElement).value);
                }}
                class="rounded border border-surface-200 bg-white px-2 py-1.5 text-sm text-slate-900"
              >
                <option value="">---</option>
                {inputKinds.map((kind) => (
                  <option key={kind} value={kind}>
                    {kind}
                  </option>
                ))}
              </select>
            )}
          </div>
          <div class="flex flex-col gap-1">
            <label class="text-sm text-slate-600">Input Name</label>
            <input
              type="text"
              value={newInputName}
              onInput={(event) => {
                setNewInputName((event.target as HTMLInputElement).value);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  void handleCreateInput();
                }
              }}
              placeholder="Input Name"
              class="rounded border border-surface-200 bg-white px-2 py-1.5 text-sm text-slate-900 placeholder:text-slate-400"
            />
          </div>
          {newInputKind === "video_capture_device" && (
            <div class="flex flex-col gap-1">
              <label class="text-sm text-slate-600">Camera</label>
              {loadingDevices ? (
                <div class="text-sm text-slate-600">Loading...</div>
              ) : (
                <select
                  value={inputSettings.device_id}
                  onChange={(event) => {
                    handleDeviceChange((event.target as HTMLSelectElement).value);
                  }}
                  class="rounded border border-surface-200 bg-white px-2 py-1.5 text-sm text-slate-900"
                >
                  <option value="">---</option>
                  {deviceItems.map((device) => (
                    <option key={device.itemValue} value={device.itemValue}>
                      {device.itemName}
                    </option>
                  ))}
                </select>
              )}
            </div>
          )}
          {newInputKind === "audio_capture_device" && (
            <DeviceSelectField
              label="Microphone"
              settingsKey="device_id"
              inputSettings={inputSettings}
              setInputSettings={setInputSettings}
              devices={deviceItems}
              loading={loadingDevices}
            />
          )}
          {newInputKind === "image_source" && (
            <InputSettingsField
              label="File"
              settingsKey="file"
              placeholder="画像ファイルパス"
              inputSettings={inputSettings}
              setInputSettings={setInputSettings}
            />
          )}
          {newInputKind === "mp4_file_source" && (
            <InputSettingsField
              label="Path"
              settingsKey="path"
              placeholder="MP4 ファイルパス"
              inputSettings={inputSettings}
              setInputSettings={setInputSettings}
            />
          )}
          {newInputKind === "rtmp_inbound" && (
            <div class="flex flex-col gap-3">
              <InputSettingsField
                label="Input URL"
                settingsKey="inputUrl"
                placeholder="rtmp://..."
                inputSettings={inputSettings}
                setInputSettings={setInputSettings}
              />
              <InputSettingsField
                label="Stream Name"
                settingsKey="streamName"
                placeholder="ストリーム名"
                inputSettings={inputSettings}
                setInputSettings={setInputSettings}
              />
            </div>
          )}
          {newInputKind === "srt_inbound" && (
            <div class="flex flex-col gap-3">
              <InputSettingsField
                label="Input URL"
                settingsKey="inputUrl"
                placeholder="srt://..."
                inputSettings={inputSettings}
                setInputSettings={setInputSettings}
              />
              <InputSettingsField
                label="Stream ID"
                settingsKey="streamId"
                placeholder="ストリーム ID"
                inputSettings={inputSettings}
                setInputSettings={setInputSettings}
              />
            </div>
          )}
          {newInputKind === "rtsp_subscriber" && (
            <InputSettingsField
              label="Input URL"
              settingsKey="inputUrl"
              placeholder="rtsp://..."
              inputSettings={inputSettings}
              setInputSettings={setInputSettings}
            />
          )}
          <div class="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => {
                void cleanupProbeInput();
                setShowCreateModal(false);
              }}
              class="rounded bg-surface-200 px-3 py-1 text-sm text-slate-600 hover:bg-surface-300"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => {
                void handleCreateInput();
              }}
              disabled={newInputName.trim() === "" || newInputKind === ""}
              class="rounded bg-accent-600 px-3 py-1 text-sm text-white hover:bg-accent-500 disabled:opacity-50"
            >
              Add
            </button>
          </div>
        </div>
      </ObsDcModal>

      {/* ソース削除確認モーダル */}
      <ObsDcModal
        open={showRemoveModal}
        title="Remove Source"
        onClose={() => {
          setShowRemoveModal(false);
        }}
      >
        <div class="flex flex-col gap-3">
          <p class="text-sm text-slate-600">
            Remove source{" "}
            <span class="font-medium text-slate-900">
              &quot;{selectedItem?.sourceName ?? ""}&quot;
            </span>{" "}
            from the scene?
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
                void handleRemoveItem();
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
