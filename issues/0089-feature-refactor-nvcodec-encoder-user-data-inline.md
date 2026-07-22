# NvcodecEncoder の SharedInputQueue を廃止して shiguredo_nvcodec の user_data 経由に一本化する

- Priority: Low
- Created: 2026-07-22
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-nvcodec-encoder-user-data-inline
- Reporter: @sile
- Decision Owner: @sile

## 目的

`src/encoder/nvcodec.rs` の `NvcodecEncoder` は、 encoder callback で入力フレームの metadata (timestamp / size) を復元するために `Arc<Mutex<VecDeque<VideoFrame>>>` 型の `SharedInputQueue` を保持している。 これは shiguredo_nvcodec crate の user_data 経路 (`Encoder::encode` の第 3 引数、 `EncodedFrame<T>::user_data()` で callback 側から取得可能) を使えば不要になる。

nvcodec decoder (`src/decoder/nvcodec.rs`) は既に user_data 経路を使っており (`self.inner.decode(&data, frame.to_stripped())`)、 encoder 側も対称化することで:

- `SharedInputQueue` 型と関連コード (~20-30 行) を削除できる
- Mutex / VecDeque への hisui 側依存を減らせる (シンプル化)
- Ok / Err の順序保証を hisui 側の queue から crate 内部の Job FIFO に一本化できる
- issues/0085 で追加した「callback Err 分岐で `pop_front` を呼ぶ順序保証 contract」も削除できる (crate 側で Job drop で user_data も自動 drop されるため)

依存: issues/0085 (in-flight bp) merge 後に着手する。 0085 の bp 導入と本 refactor はコード的に独立だが、 0085 の中で SharedInputQueue に関連する変更 (Err 分岐 pop_front 追加、 順序保証コメント更新) が入っており、 0085 未 merge の状態で本 refactor を着手すると conflict しやすい。

## 優先度根拠

Low。 挙動を変えない refactor で、 バグ修正でも機能追加でもない。

- decoder と encoder の対称化で読み手にとっての mental model が揃う
- LOC 削減 (~20-30 行) で保守性向上
- 0085 完了後に着手可能、 緊急性なし

## 現状

`src/encoder/nvcodec.rs`:

- `type SharedInputQueue = Arc<Mutex<VecDeque<VideoFrame>>>;` (metadata FIFO)
- `NvcodecEncoder::input_queue: SharedInputQueue` フィールド
- `NvcodecEncoder::inner: Encoder<FnEncodeHandler<(), Error>>` の user_data 型が `()`
- `encode()` 内で `input_queue.lock().push_back(video_frame.to_stripped())` して metadata を保持
- callback (`build_handler`) の Ok 分岐で `input_queue.lock().pop_front()` して metadata を復元
- callback の Err 分岐でも `input_queue.lock().pop_front()` (0085 で追加した順序保証 contract)
- 順序保証コメント (`nvcodec.rs:342-344`) で「Mutex 排他 + VecDeque FIFO + crate 内部 worker FIFO」を説明

`src/decoder/nvcodec.rs` (対称化の参考):

- `NvcodecDecoder::inner: Decoder<FnDecodeHandler<VideoFrame, Error>>` の user_data 型が `VideoFrame`
- `decode()` 内で `self.inner.decode(&data, frame.to_stripped())` として user_data を crate に渡す
- callback で `decoded.into_parts()` から user_data (input_frame) を復元

crate `shiguredo_nvcodec 2026.2.0` の該当 API:

- `Encoder<H: EncodeHandler>::encode(&self, data: &[u8], options: &EncodeOptions, user_data: H::UserData)` (`encode.rs:1288-1301`)
- `EncodedFrame<T>::user_data(&self) -> &T` / `into_parts(self) -> (Vec<u8>, T)` (`encode.rs:1423-1456`)
- Job::Encode に user_data を保持し、 worker が FIFO で処理 → callback で `EncodedFrame` として返す
- Err 時は Job が drop され、 user_data も自動 drop (hisui 側で明示 pop 不要)

## 設計方針

以下は polish で確定する。 現時点では骨組みのみ。

### user_data 経由への切り替え

- `NvcodecEncoder::inner` の型を `Encoder<FnEncodeHandler<VideoFrame, Error>>` に変更 (user_data 型を `()` → `VideoFrame` に)
- `NvcodecEncoder::input_queue: SharedInputQueue` フィールドを削除
- `SharedInputQueue` 型 alias 削除、 `use std::collections::VecDeque;` / `use std::sync::Mutex;` の import 整理

### build_handler の signature 変更

- `input_queue: SharedInputQueue` パラメータを削除
- callback の Ok 分岐: `encoded_frame.into_parts()` → `(data, input_frame)` として metadata 取得
- callback の Err 分岐: `input_queue.pop_front()` 呼び出しを削除 (crate 側の Job drop で自動 drop)
- 「encoded frame produced without input frame」エラーパスは不要 (user_data 型が Option ではないため必ず存在する)

### encode() の変更

- `input_queue.lock().push_back(video_frame.to_stripped())` 削除
- `self.inner.encode(&nv12_data, &encode_options, video_frame.to_stripped())` に変更 (第 3 引数を `()` → `to_stripped()`)
- 順序保証コメント (`nvcodec.rs:342-344`) を削除または「crate 内部 worker FIFO に委譲」に更新

### build_encoder の変更

- `let input_queue: SharedInputQueue = Arc::new(Mutex::new(VecDeque::new()));` 削除
- `build_handler` 呼び出しから `input_queue.clone()` 削除
- `Self { ..., input_queue, ... }` から `input_queue` フィールドを削除

## 完了条件

polish で確定。 主要項目 (骨組み):

- `src/encoder/nvcodec.rs::NvcodecEncoder::inner` の user_data 型が `VideoFrame`
- `SharedInputQueue` 型 alias と `input_queue` フィールドが削除されている
- `build_handler` が user_data 経由で input frame を取得している
- `encode()` の第 3 引数が `video_frame.to_stripped()` 相当
- callback Err 分岐に `pop_front` 呼び出しがない (crate 側自動管理)
- 順序保証コメントが更新されている (Mutex + VecDeque の記述を削除)

### grep 検証

- `rg 'SharedInputQueue' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'input_queue' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'Arc<Mutex<VecDeque' src/encoder/nvcodec.rs` の hit が **0 件**

### cargo

polish で確定 (0085 §完了条件 §cargo と同型)。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

内部 refactor で挙動不変のため、 CHANGES.md 記載は不要 (polish で再判断)。

## 関連

- issues/0085 (`feature/refactor-encoder-inflight-backpressure`): 依存先。 0085 merge 後に着手する。 0085 で追加した「callback Err 分岐 pop_front」と「順序保証コメント」は本 issue で自然に削除される
- issues/0087 (`feature/add-realtime-encoder-param-override`): 0085 の後続。 0087 と本 issue は独立 (順不同で着手可)
- `src/decoder/nvcodec.rs`: 対称化の参考実装
