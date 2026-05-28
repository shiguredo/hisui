import type { Signal } from "@preact/signals";
import { useRef, useState } from "preact/hooks";
import type { ConnectionState, BootstrapConfig } from "../p2p/types.ts";
import { supportsAlwaysNegotiateDataChannels } from "../p2p/client.ts";

interface ConnectionPanelProps {
  connectionState: Signal<ConnectionState>;
  onConnect: (config: BootstrapConfig) => void;
  onDisconnect: () => void;
}

const DEFAULT_BOOTSTRAP_URL = "http://127.0.0.1:4455/bootstrap";

export function ConnectionPanel({
  connectionState,
  onConnect,
  onDisconnect,
}: ConnectionPanelProps) {
  const stunInputRef = useRef<HTMLInputElement>(null);
  const urlInputRef = useRef<HTMLInputElement>(null);
  const [stunVisible, setStunVisible] = useState(false);
  const alwaysNegotiateSupported = supportsAlwaysNegotiateDataChannels();
  const [dataChannelOnly, setDataChannelOnly] = useState(alwaysNegotiateSupported);
  const state = connectionState.value;
  const isActive = state === "bootstrapping" || state === "connecting" || state === "connected";
  const isDisabled =
    state === "bootstrapping" || state === "connecting" || state === "disconnecting";

  function handleClick(): void {
    if (isActive) {
      onDisconnect();
    } else {
      const bootstrapUrl = urlInputRef.current?.value.trim() ?? DEFAULT_BOOTSTRAP_URL;
      const stunUrl = stunInputRef.current?.value.trim() ?? "";
      const iceServers = stunUrl !== "" ? [{ urls: stunUrl }] : undefined;
      onConnect({ bootstrapUrl, iceServers, dataChannelOnly });
    }
  }

  const buttonLabel = isActive ? "Disconnect" : "Connect";

  return (
    <div class="flex flex-col gap-3">
      <label class="text-base font-medium text-slate-800">STUN Server</label>
      <div class="relative">
        <input
          ref={stunInputRef}
          type={stunVisible ? "text" : "password"}
          defaultValue={import.meta.env.VITE_STUN_SERVER_URL ?? ""}
          disabled={isActive}
          class="field-control w-full px-3 py-2 pr-10 text-base"
        />
        <button
          type="button"
          onClick={() => {
            setStunVisible(!stunVisible);
          }}
          class="absolute top-1/2 right-2 -translate-y-1/2 p-1 text-slate-500 hover:text-slate-800"
        >
          <svg
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            {stunVisible ? (
              <>
                <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                <circle cx="12" cy="12" r="3" />
              </>
            ) : (
              <>
                <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
                <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
                <path d="M14.12 14.12a3 3 0 1 1-4.24-4.24" />
                <line x1="1" y1="1" x2="23" y2="23" />
              </>
            )}
          </svg>
        </button>
      </div>
      <label class="flex items-center gap-2 text-base font-medium text-slate-800">
        <input
          type="checkbox"
          checked={dataChannelOnly}
          disabled={isActive || !alwaysNegotiateSupported}
          onChange={(event) => {
            setDataChannelOnly((event.target as HTMLInputElement).checked);
          }}
          class="h-4 w-4 rounded border-surface-300 bg-white accent-accent-500 disabled:opacity-50"
        />
        DataChannel Only
        {!alwaysNegotiateSupported && <span class="text-sm text-slate-500">(not supported)</span>}
      </label>
      <label class="text-base font-medium text-slate-800">Bootstrap URL</label>
      <input
        ref={urlInputRef}
        type="text"
        defaultValue={DEFAULT_BOOTSTRAP_URL}
        disabled={isActive}
        class="field-control px-3 py-2 text-base"
      />
      <button
        type="button"
        onClick={handleClick}
        disabled={isDisabled}
        class="w-full rounded-md bg-accent-600 px-4 py-2 text-base font-medium text-white shadow-sm hover:bg-accent-500 disabled:opacity-50"
      >
        {buttonLabel}
      </button>
    </div>
  );
}
