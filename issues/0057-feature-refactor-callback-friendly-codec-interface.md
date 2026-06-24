# エンコーダー / デコーダーのインターフェースを callback friendly に再設計する検討

- Priority: Medium
- Created: 2026-06-24
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-callback-friendly-codec-interface
- Polished: {YYYY-MM-DD}
- Reporter: @sile

## 目的

`shiguredo_nvcodec 2026.2.0` のような **非同期コールバック型** のエンコーダー / デコーダー実装を、hisui 側の上位パイプラインでもそのまま活かせる設計を検討する。

現状の hisui 上位インターフェース (`VideoEncoder` / `VideoDecoder`) は同期 pull 型 (`encode` → `next_encoded_frame()` / `poll_output()`) に固定されている。下位ライブラリ側の非同期コールバックを受け止めるには、いったん共有キューへ push してから上位 pull 側で取り出す経路を挟む必要があり、フレーム単位のラウンドトリップが固定で挟まる構造になっている。これを上位パイプライン全体で見直し、callback / push ベースの経路を素直に流せるようにすることが本 issue のゴール。

## 優先度根拠

Medium とする。

- 現時点で性能に明確な悪影響を測ったわけではなく、ユーザー報告由来の不具合でもない。
- ただし上流 (`shiguredo_nvcodec` 等) が今後ますます非同期コールバック寄りに進化する見込みが高く、hisui 側が同期 pull のままだとそのたびにアダプター層を再実装することになる。
- 設計検討フェーズで方向性を固めれば、後続の実装 issue を最小コストで進められる。

## 現状

### 非同期 API への薄いアダプターになっている例

`src/encoder/nvcodec.rs` および `src/decoder/nvcodec.rs` は `shiguredo_nvcodec 2026.2.0` のコールバック API に追従する形で実装されている。具体的には、

- `shiguredo_nvcodec::Encoder::new(config, handler)` の `handler` に `FnEncodeHandler<()>` を渡し、ワーカースレッドから `Arc<Mutex<VecDeque<EncodedFrame<()>>>>` に push する。
- 上位の `NvcodecEncoder::encode()` 直後に `handle_encoded_frames()` を呼び、キューから pop して `output_queue: VecDeque<VideoFrame>` に並べ直す。
- 同じ構造を `NvcodecDecoder` でも採用している (`decoded_queue` 経由)。

ここで `Arc<Mutex<VecDeque<_>>>` を一段挟んでいるため、

- 下位ワーカースレッドが完了通知した瞬間に hisui の次段が即時に動けるわけではなく、上位スレッドが次に `handle_encoded_frames()` / `handle_decoded_frames()` を呼ぶまで待たされる。
- 1 フレーム投入につき 1 フレーム取り出すような同期的呼び出しに最適化されており、複数フレームの先行投入 / バッチ取り出しを上位が活用しにくい。
- ハンドラ内で発生したエラーは `error_slot` に保持しておき、次回 `handle_*_frames()` 呼び出しで初めて伝搬する。即時通知ができない。

### 上位インターフェースが同期 pull 前提

- `src/encoder.rs` の `VideoEncoder` は `encode()` → 内部 `output_queue` への詰め込み → `next_encoded_frame()` で 1 個ずつ取得する pull 型。
- `src/decoder.rs` の `VideoDecoder` は `handle_input_sample()` → `poll_output()` という pull 型。
- 上位パイプライン（mixer / writer 系）もこの pull 型を前提に書かれている。

つまり、hisui のパイプライン全体が「同期 pull」を前提にしており、callback friendly な下位 API を活かすためには **上位の interface 再設計が必要** という構造になっている。

## 設計方針

設計検討フェーズなので、本 issue では以下の方向性を比較・評価して結論を出す。実装は別 issue で扱う。

### A. 上位 interface に `Sink<EncodedFrame>` / `Sink<DecodedFrame>` を渡せるようにする

- `VideoEncoder::new` で出力の押し出し先 (`tokio::sync::mpsc::Sender` 相当) を渡し、エンコード完了は下位コールバックから直接そこに push する。
- 上位パイプラインは `Receiver` 側を `poll_output` の代わりに使う。
- 利点: `shiguredo_nvcodec` のコールバックを最短経路で次段に届けられる。
- 課題: 同期 pull 型エンコーダー (`libvpx`, `openh264`, `svt_av1`, 等) は `encode()` の戻りで全フレームを取れるので、それを Sink に push する薄いラッパーが要る。

### B. `VideoEncoder` を trait オブジェクト風にして、コールバック型と pull 型の両方を許容する

- 既存の `next_encoded_frame()` 路を残しつつ、`with_callback(...)` のような副 API を追加して、上位が選べるようにする。
- 利点: 段階的に移行可能。
- 課題: 2 系統を維持するメンテコストが増える。

### C. tokio チャネルを中核に据えて、全エンコーダーで非同期化する

- 同期 pull 型エンコーダーも内部で worker thread を起動して、`Receiver` 越しに出力する形に揃える。
- 利点: 上位パイプラインからは全て対称になる。
- 課題: 軽量エンコーダーに不要なスレッドが入る。`tokio` を encoder 層に持ち込むかどうかも要検討。

評価軸:

- **遅延**: フレームが下位コールバックで返ってから上位パイプライン次段に渡るまでのホップ数。
- **エンコーダー実装側のコスト**: 既存実装 (`libvpx`, `openh264`, `svt_av1`, `video_toolbox`, `audio_toolbox`, `fdk_aac`, `opus`) への波及。
- **テスト容易性**: hisui の規約 (`shiguredo-rust`) 上、モックは禁止なので、再設計後もテストで実体を組み合わせて検証できる構造であること。
- **依存関係**: `tokio` を encoder 層まで広げるかどうか、`async fn` を採用するかどうか。

## 完了条件

- 上記 A / B / C もしくは他の選択肢から 1 つに結論を出し、本 issue 内に決定理由を追記してある。
- 実装に着手するための前提（`tokio` を持ち込むか、`Sink` / `Sender` の型を何にするか、エラー伝搬の経路など）が明文化されている。
- 後続の実装 issue を切れる粒度まで分解されている。

## 解決方法

### 1. 現状調査

- 影響範囲を網羅: `src/encoder.rs` / `src/decoder.rs` の `VideoEncoder` / `VideoDecoder` 利用箇所、および `src/mixer/`, `src/writer/`, `src/processor.rs` 周辺で `next_encoded_frame()` / `poll_output()` を呼んでいる箇所を洗い出す。
- 各エンコーダー / デコーダー実装 (`src/encoder/*.rs`, `src/decoder/*.rs`) ごとに、内部で非同期スレッドを持っているかどうか・現在の出力モデル（同期 pull / コールバック / Future）を整理する。

### 2. 設計案の検証

- A / B / C それぞれについて、`src/encoder.rs` の `VideoEncoder::new` まわりと、上位の `processor.rs` / `mixer/video.rs` の呼び出し側で必要な変更量を見積もる。
- 1 案を選んで、簡単なプロトタイプ（`NvcodecEncoder` 周辺のみ書き換えるレベル）で遅延・スループットを定性的に確認する。

### 3. 決定

- 採用案・棄却理由・後続実装 issue の分割粒度を本 issue 内に追記する。

なお、上記検討に伴う実装は本 issue では行わず、別 issue として切り出す。
