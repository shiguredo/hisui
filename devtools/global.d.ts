// CSS ファイルの side-effect import を許可する
declare module "*.css" {}

// Vite の環境変数の型定義
interface ImportMetaEnv {
  readonly DEV: boolean;
  readonly PROD: boolean;
  readonly VITE_STUN_SERVER_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

// Chrome M146 の alwaysNegotiateDataChannels は TypeScript の lib.dom.d.ts に未定義
interface RTCConfiguration {
  alwaysNegotiateDataChannels?: boolean;
}

// MediaStreamTrackProcessor は TypeScript の lib.dom.d.ts に未定義
interface MediaStreamTrackProcessorInit {
  track: MediaStreamTrack;
  maxBufferSize?: number;
}

declare class MediaStreamTrackProcessor {
  readonly readable: ReadableStream<VideoFrame>;
  constructor(init: MediaStreamTrackProcessorInit);
}
