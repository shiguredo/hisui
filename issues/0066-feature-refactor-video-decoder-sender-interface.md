# AsyncVideoDecoder を追加し VideoDecoder を内部 channel ベースに改修する

- Priority: Medium
- Created: 2026-06-26
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-add-async-video-decoder
- Polished:
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0057 で確定した採用案 C (全エンコーダー / デコーダーを Sender 経由の出力に統一) を、**段階的に移行可能な形で** VideoDecoder 系に適用する。

具体的には:

1. **`AsyncVideoDecoder` を新規追加**: 内部 channel から `recv().await` でフレームを受け取る非同期インターフェース
2. **既存 `VideoDecoder` (同期) を内部 channel ベースに改修**: 旧 `VecDeque<VideoFrame>` を `tokio::sync::mpsc::UnboundedSender` / `UnboundedReceiver` に置き換え、`next_decoded_frame()` は `Receiver::try_recv()` で取り出す形にする。**外部 API (同期 pull 型) は維持** し、既存使用側の書き換えは不要
3. **inner 層は AsyncVideoDecoder / VideoDecoder の両方で共有**: 各 inner (`LibvpxDecoder` / `Openh264Decoder` / `Dav1dDecoder` / `VideoToolboxDecoder` / `NvcodecDecoder`) はコンストラクタで `Sender` を受け取って内包し、`decode()` 内で出力フレームを Sender に push する形に統一。同期 inner は `tx.try_send()` で push (内部 channel は unbounded のため失敗しない)、非同期 inner (Nvcodec) は callback 内で push

これにより、本 issue では **既存使用側 (compose / vmaf / inspect / mp4 reader / rtmp / rtsp / srt / obsws/file_mp4) の書き換えは一切不要** になる。各使用側の `AsyncVideoDecoder` への移行は **別 issue で段階的に** 行う。

詳細な動機・採用理由は closed/0057 §3 参照。closed/0057 の採用案 C で禁じていた「中途半端な 2 系統共存」は本方針では **意図的に許容** する (移行コストを段階分けする戦略)。closed/0057 §3 分割表も方針変更を反映するために更新が必要。

## 優先度根拠

Medium。

- closed issue 0057 で採用案が確定済み。実装着手段階に入っているため Medium 維持
- 依存先: 0067 (encoder) は本 issue 完了後に同パターンで対称展開 (`AsyncAudioEncoder` も同様)
- 本方針 (δ) は polish 段階の (α) / (β) / (γ) を踏まえて、Decision Owner の判断で確定した第 4 案。既存使用側を 1 つも壊さず Async 系インターフェースを追加できるため、段階的移行が安全
- 採用案 C 再検討トリガー: 内部 channel ベース改修が同期 API の挙動 (`next_decoded_frame` の戻り値、`finish` のタイミング等) を変えてしまう場合は、対応方法を Decision Owner が判断

## 現状

`src/decoder.rs` および `src/decoder/*.rs` の各 inner の構造は closed issue 0057 「現状」§の表を参照。本 issue 固有の論点は以下:

### 既存 VideoDecoder の内部構造

- `VideoDecoder.decoded: VecDeque<VideoFrame>` (`src/decoder.rs:335`) で同期 pull
- `VideoDecoder::next_decoded_frame()` 系 dispatch (実体は `inner.next_decoded_frame()` → `output_queue.pop_front()`)
- `VideoDecoder::poll_output()` (`src/decoder.rs:422-430`) で `MediaFrame::video(frame)` に包んで返す
- `drain_video_decoder_output` (`src/decoder.rs:514-533`) で `VideoDecoder::poll_output()` を回しながら `TrackPublisher::send_media()` に流す

これら同期 pull API は **全て維持する**。内部のフレーム保持構造を `VecDeque` から `tokio::sync::mpsc::UnboundedReceiver` に置き換えるだけ。`next_decoded_frame()` の戻り値型 (`Option<VideoFrame>`) と挙動 (queue が空なら `None`) は不変。

### 各 inner の現状出力モデル

| 実装 | 内部 API | 内部キュー | 備考 |
|------|----------|------------|------|
| `LibvpxDecoder` | 同期 | `input_queue` + `output_queue` | `decode()` 内で `handle_decoded_frames()` を呼ぶ |
| `Openh264Decoder` | 同期 | `input_queue` + `output_queue` | keyframe 入力時に `finish()` 経由で 0〜2 フレーム送信 |
| `Dav1dDecoder` | 同期 | `input_queue` + `output_queue` | |
| `VideoToolboxDecoder` | 非同期 | `decoded: Option<VideoFrame>` 単発 | `shiguredo_video_toolbox` 内で std::sync::mpsc 化済み |
| `NvcodecDecoder` | 非同期 | `decoded_queue: Arc<Mutex<VecDeque>>` + `input_queue` + `error_slot` | hisui コードが `FnDecodeHandler` を直接実装 |

各 inner の `output_queue` / `decoded_queue` (および `next_decoded_frame()` API) を廃止し、代わりに **コンストラクタで `tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>` を受け取って内包** する形に統一する。同期 inner では `decode()` 内で同期的に `tx.try_send()` する (unbounded なので失敗しない)。非同期 inner (Nvcodec) では callback 内から `tx.send()` する (unbounded なので `block_in_place` 不要)。

### 外部利用箇所 (本 issue では一切書き換えない)

- `drain_video_decoder_output` pull 型: `src/mp4/reader.rs:1236, 1286` (2 行) / `src/rtsp/subscriber.rs:662` / `src/srt/inbound_endpoint.rs:445` / `src/rtmp/inbound_endpoint.rs:422`
- `discard_video_decoder_output` (`src/mp4/reader.rs:1388`)
- `VideoDecoder::new` 外部生成 (9 call sites)
- `set_video_decoder` (`src/obsws/source/file_mp4.rs:61` → `Mp4FileReader::set_video_decoder` `src/mp4/reader.rs:318`)

これらは **すべて現状のまま動く**。`VideoDecoder::new` のシグネチャは変えず、`next_decoded_frame()` / `poll_output()` も挙動不変。

### メトリクス計上の現状

- `total_input_video_frame_count_metric.inc()` (`src/decoder.rs:333, 345, 405`): 入力フレーム数
- `total_output_video_frame_count_metric.inc()` (`src/decoder.rs:334, 346, 415`): 出力フレーム数

これらの計上ポイントも維持する (内部構造を `VecDeque` から `UnboundedReceiver` に置き換えるだけ)。

## 設計方針

### 内部 channel を core にした 2 系統 API 提供

```
       +--------------+
input  |  各 inner    |   tx (内部 channel)         rx (内部 Receiver)
-----> | decode/finish| ---------------------+----> +-----------+
       |   (sync fn)  |                      |      | VideoDecoder       <-- 同期 API (既存維持)
       +--------------+                      |      |   next_decoded_frame() = rx.try_recv()
                                             |      +-----------+
                                             |
                                             +----> +-----------+
                                                    | AsyncVideoDecoder  <-- 非同期 API (新規)
                                                    |   recv().await
                                                    +-----------+
```

- **内部 channel**: `tokio::sync::mpsc::unbounded_channel::<crate::Result<VideoFrame>>()` (unbounded で `try_send` 失敗なし、`block_in_place` 不要)
- **`VideoDecoder` (同期、既存維持)**: 内部に `rx: UnboundedReceiver` を持ち、`next_decoded_frame()` は `self.rx.try_recv().ok().and_then(|r| r.ok())` で実装。エラーの伝搬は次節で議論
- **`AsyncVideoDecoder` (新規)**: 内部に `rx: UnboundedReceiver` を持ち、`next_decoded_frame_async()` (仮称) は `self.rx.recv().await` で実装
- **共有部分**: 内部 inner (`VideoDecoderInner` enum) と Sender / Receiver の所有関係を共通化する。`VideoDecoder` と `AsyncVideoDecoder` で構造体定義を共有する形 (一方が他方を内包する) もありうる

### 構造体設計の選択肢

**選択肢 (1): `VideoDecoder` を core にして `AsyncVideoDecoder` が `VideoDecoder` を包む**

```rust
pub struct VideoDecoder {
    inner: VideoDecoderInner,
    rx: UnboundedReceiver<crate::Result<VideoFrame>>,
    // tx は inner に渡し済み (構造体には保持しない)
    // metrics 等
}

pub struct AsyncVideoDecoder {
    inner_decoder: VideoDecoder,  // wrap
}

impl AsyncVideoDecoder {
    pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.inner_decoder.rx.recv().await
    }
}
```

**選択肢 (2): `AsyncVideoDecoder` を core にして `VideoDecoder` が wrap**

```rust
pub struct AsyncVideoDecoder {
    inner: VideoDecoderInner,
    rx: UnboundedReceiver<crate::Result<VideoFrame>>,
}

pub struct VideoDecoder {
    inner_decoder: AsyncVideoDecoder,
}

impl VideoDecoder {
    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.inner_decoder.rx.try_recv().ok().and_then(|r| r.ok())
    }
}
```

**選択肢 (3): 2 つの独立構造体 + 共通の inner ヘルパ trait**

両者を独立構造体として保持し、共通の inner trait (`VideoDecoderInner`) を持つ。

実装着手段階で Decision Owner が選択する。シンプル性で言えば (1) または (2)。

### 各 inner の Sender 化形態

`VideoDecoderInner` の各 variant をコンストラクタで `tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>` を受け取って内包する形に統一する。

- **同期 inner (Libvpx / Openh264 / Dav1d)**: `decode()` は **同期 fn** のまま。内部で出力フレームを `self.tx.send(Ok(frame))` で push (unbounded なので失敗しない)。await は不要
- **VideoToolboxDecoder**: `decode()` 同期 fn のまま。`decoded: Option<VideoFrame>` を廃止して直接 `self.tx.send(Ok(frame))`。`reinitialize_if_need` の `decoded.is_some()` ガードは不要 (sequential 呼出で frame が積まれる順序は保たれる)
- **NvcodecDecoder**: `decode()` 同期 fn のまま。`FnDecodeHandler` 内で `tx.send(Ok(frame))` を直接呼ぶ (unbounded なので `block_in_place + blocking_send` 不要)。`error_slot` / `decoded_queue` は廃止し、callback 内エラーは `tx.send(Err(_))` で即時通知

inner の `decode` / `finish` シグネチャは現状の同期 fn を維持 (`async fn` 化不要、`block_on` も不要、tokio runtime context 依存なし)。

### unbounded channel 採用の正当化

- inner は frame ごとに 1〜2 フレーム push するだけで、 突発的に大量送信することはない
- 内部 channel が `unbounded` でも実用上のメモリ使用量増加は小さい (raw I420 1080p で 1 フレーム約 3MB、通常は数フレーム以内に消費)
- バックプレッシャは下流の `output_tx: TrackPublisher` の `send_media` で発生 (broadcast の lag drop 経由)
- `bounded` channel + `block_in_place + blocking_send` の複雑性を避け、シンプル性を優先

### エラー伝搬

- `Result<VideoFrame, crate::Error>` を Sender に流す形に統一
- 同期 inner の `decode()` 中で発生したエラーは、`decode()` の戻り値で `Err` を返す形を維持 (Sender には流さない)
- 非同期 inner (Nvcodec) の callback 内エラーは Sender 経由で `Err(_)` を即時通知 (`error_slot` 廃止)
- 同期 `VideoDecoder::next_decoded_frame()` は `Option<VideoFrame>` を返す既存挙動を維持: `rx.try_recv()` で取得した `Result<VideoFrame, Error>` のうち `Err` は無視 (or log) して `None` を返す。エラー伝搬は別経路 (例: 次回の `decode()` 呼出時に蓄積 Err を返す) で行うか、`poll_output()` の戻り値で `Err` を返すか、Decision Owner が選択

### shiguredo-rust 規約整合

- トレイト追加なし (`VideoDecoderInner` enum を維持)
- `#[non_exhaustive]` 不使用
- モック / スタブ不使用 (テストは実 decoder + tokio channel)
- 規約上の許可取得は不要

## 完了条件

- `AsyncVideoDecoder` 構造体が `src/decoder.rs` に新規追加され、 `next_decoded_frame_async() -> Option<crate::Result<VideoFrame>>` (or 類似の async API) を提供する
- 既存 `VideoDecoder` の内部実装が `VecDeque` から `tokio::sync::mpsc::unbounded_channel` ベースに切り替わっている
- 既存 `VideoDecoder::next_decoded_frame()` / `VideoDecoder::poll_output()` / `drain_video_decoder_output` の **外部 API 挙動は不変** (戻り値型・タイミング・エラー伝搬経路すべて維持)
- 各 inner (`Libvpx` / `Openh264` / `Dav1d` / `VideoToolbox` / `Nvcodec`) のコンストラクタが `tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>` を受け取って内包する形に変更されている
- 各 inner の `output_queue` / `decoded_queue` / `error_slot` / `decoded: Option<VideoFrame>` 等の中継キューが廃止されている
- 各 inner の `next_decoded_frame()` 系 API が廃止されている
- 各 inner の `decode()` / `finish()` シグネチャは同期 fn のまま維持 (async fn 化なし)
- `MediaPipeline` / `ProcessorHandle` 経由の既存使用側 (compose / vmaf / inspect / mp4 reader / rtmp / rtsp / srt / obsws/file_mp4) は **一切書き換えない** (バイナリ互換維持を確認)
- 新規 end-to-end テスト (`src/decoder/nvcodec.rs` 末尾) が追加され、以下を検証:
  - (a) `NvcodecDecoder` で callback 内 `Err` が次回 `try_recv()` で取得できる
  - (b) `Openh264Decoder` で keyframe 入力時に `finish()` 経由の旧フレーム + 新フレームが両方 Receiver に届く
  - (c) `AsyncVideoDecoder::next_decoded_frame_async()` で正常に `recv().await` できる
- 既存 `src/decoder.rs:720-821` のエンジン選択テストは現状形 (同期 `decoder.handle_input_sample(...)`) のまま動く
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

### 1. 内部 channel 型の確定

`type DecoderOutputSender = tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>;` のような型エイリアスを `src/decoder.rs` に追加。各 inner のコンストラクタで受け取る型を統一する。

### 2. inner の Sender 化 (同期 fn のまま)

各 inner のコンストラクタを `new(..., tx: DecoderOutputSender) -> crate::Result<Self>` 形に変更。`output_queue` / `decoded_queue` / `error_slot` / `decoded: Option<VideoFrame>` を廃止。

- **同期 inner (Libvpx / Openh264 / Dav1d)**: `decode()` 内で `self.tx.send(Ok(frame)).expect("unbounded channel never closed")` の形で push (send は unbounded なので `closed` のみが Err、その場合は VideoDecoder が drop された後 = bug なので panic で良い)
- **VideoToolboxDecoder**: 同様に `decode()` 同期 fn 内で `self.tx.send(Ok(frame))`、`reinitialize_if_need` の `decoded.is_some()` ガードを廃止
- **NvcodecDecoder**: `FnDecodeHandler` 内で `tx.send(Ok(frame))` / `tx.send(Err(_))` を直接呼ぶ。`input_queue` は callback と `decode()` push 側で共有 (`Arc<Mutex<>>` 化)、`error_slot` / `decoded_queue` 廃止

### 3. VideoDecoder の内部 channel 化

`VideoDecoder` 構造体に `rx: tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>` を追加 (`decoded: VecDeque<VideoFrame>` を置換)。

- `VideoDecoder::new(options, stats)` 内で `let (tx, rx) = tokio::sync::mpsc::unbounded_channel();` を作成、tx を inner に渡す
- `VideoDecoder::next_decoded_frame()`: 戻り値型 `Option<VideoFrame>` を維持しつつ、実装を `self.rx.try_recv().ok().and_then(|r| r.ok())` 等に変更 (エラーの扱いは §設計方針エラー伝搬で確定)
- `VideoDecoder::poll_output()`: 既存挙動を維持 (内部実装だけ変更)
- `drain_video_decoder_output`: 既存挙動を維持
- `handle_input_sample` / `handle_input_message`: 既存挙動を維持

### 4. AsyncVideoDecoder の新規追加

設計方針 §構造体設計の選択肢から (1) / (2) / (3) を選択。本 issue 起票時の暫定推奨は (2) (`AsyncVideoDecoder` core + `VideoDecoder` wrap)。

```rust
pub struct AsyncVideoDecoder {
    inner: VideoDecoderInner,
    rx: tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
    // metrics 等
}

impl AsyncVideoDecoder {
    pub fn new(options: VideoDecoderOptions, stats: crate::stats::Stats) -> Self { ... }
    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> { ... }  // 同期 fn
    pub fn finish(&mut self) -> crate::Result<()> { ... }  // 同期 fn
    pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.rx.recv().await
    }
}
```

### 5. end-to-end テスト追加

`src/decoder/nvcodec.rs` 末尾に以下を追加:

- (a) NvcodecDecoder + tokio mpsc channel で callback Err 即時通知を検証
- (b) Openh264Decoder + tokio mpsc channel で keyframe finish 順序を検証
- (c) AsyncVideoDecoder の `next_decoded_frame_async()` の正常動作を検証

### 6. 既存テストの追従

`src/decoder.rs:720-821` のエンジン選択テストは外部 API (`VideoDecoder::new` / `handle_input_sample`) の挙動が不変なので **そのまま動く** はず。動かなければ最小改変で追従する。

## 段階的移行 (本 issue では実施しない)

各使用側の `VideoDecoder` → `AsyncVideoDecoder` への移行は **別 issue で 1 箇所ずつ段階的に** 実施する。最終的に同期 `VideoDecoder` の使用箇所がゼロになった時点で `VideoDecoder` 自体を削除し、`AsyncVideoDecoder` を `VideoDecoder` にリネームする (この最終ステップも別 issue)。

移行候補の優先順位 (暫定):

1. `subcommand_inspect` (単発 decode、最も小さい影響範囲)
2. `recording_subcommand_compose` / `recording_subcommand_vmaf` (`spawn_processor` 経由)
3. mp4 reader (`Mp4FileReader::recreate_decoders` async fn 化を含む大改修)
4. RTMP / RTSP / SRT inbound endpoint (構造体改修を含む大改修)
5. obsws/source/file_mp4 (mp4 reader 改修と連動)

各移行 issue で「2 系統共存中の中間状態を 1 つずつ消していく」進め方をする。

## CHANGES.md について

内部リファクタにつき記載不要。`VideoDecoder` 系は library として外部公開していないため、API 変更の後方互換影響は本 issue ではゼロ (既存 API 挙動を維持)。後続移行 issue でも各々判断する。

## 関連

- closed/0057 (`feature/refactor-callback-friendly-codec-interface`): 設計検討の親 issue。本 issue は §3 採用案 C の decoder 部分実装。本 issue で「2 系統共存 + 段階移行」方針 (δ) を採用したため、closed/0057 §3 で禁じていた「中途半端な 2 系統共存」を意図的に許容する形になる。closed/0057 §3 分割表の更新が必要 (本 issue 完了時に追記、または別途整理)
- closed/0054 (`feature/refactor-encoder-defer-output-until-sample-entry-ready`): encoder で sample_entry 未確定時の出力を `Err` 化する fail-fast 整備。decoder 側は sample_entry 不変条件の対象外だが、本 issue でも `Result<VideoFrame>` 流路を採用
- open/0067 (`feature/refactor-video-encoder-sender-interface`): 後続 PR。本 issue で確立した「`Async*` 追加 + 同期既存維持 + 段階移行」パターンを encoder に展開する。依存順序: `0066 → 0067`
- 本 issue ブランチ: `feature/refactor-video-decoder-sender-interface` は方針 (γ) 時代の WIP 実装 (`af5f63ce` および後続 commit) を保持しているが、方針 (δ) 採用に伴い **このブランチでは継続実装しない**。新ブランチ (例: `feature/refactor-add-async-video-decoder` 仮称) を develop から切って (δ) 実装を進める。旧ブランチは参考資料として残置 (close なしに保持、最終的に方針確定後に削除判断)

## 過去の polish レビュー結果と方針 (δ) 採用に至った経緯

本 issue は当初「全使用側を Sender 化、2 系統共存を残さない」採用案 C (closed/0057 §3 確定) で polish (5 周レビュー) 済み (`ea7f1ab5 0066 polished VideoDecoder 系とその利用箇所を Sender 出力に統一する`)。

その後実装着手 (`feature/refactor-video-decoder-sender-interface` ブランチ、`af5f63ce` 他 commit) の途中で、RTMP/RTSP/SRT inbound endpoint と Mp4FileReader が想定以上に大規模な改修を要することが判明 (詳細は git log の `46626303` / `8d9d1163` commit を参照)。Decision Owner と相談した結果、以下の (δ) 方針に切り替えることになった:

- **AsyncVideoDecoder を新規追加 + 既存 VideoDecoder の内部だけ channel 化 (外部 API 維持) + 段階的移行**

これにより、本 issue では既存使用側を 1 つも変更せずに済み、最終形 (全使用側 Async) への移行は段階的に進められる。closed/0057 §3 の「中途半端な 2 系統共存禁止」とは方針が異なるため、closed/0057 §3 も併せて見直す必要がある。

本 issue 本文は (δ) 方針への切り替えに伴い 2026-06-29 に全面書き換えされた drafty 版。本格的な polish (5 周レビュー) は別途実施する必要がある (`Polished:` フィールドが空のまま)。
