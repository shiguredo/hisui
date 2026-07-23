# NvcodecEncoder の SharedInputQueue を廃止して shiguredo_nvcodec の user_data 経由に一本化する

- Priority: Low
- Created: 2026-07-22
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-nvcodec-encoder-user-data-inline
- Polished: 2026-07-23
- Reporter: @sile
- Decision Owner: @sile

## 目的

`src/encoder/nvcodec.rs` の `NvcodecEncoder` は、 encoder callback で入力フレームの metadata (timestamp / size) を復元するために `Arc<Mutex<VecDeque<VideoFrame>>>` 型の `SharedInputQueue` を保持している。 これは shiguredo_nvcodec crate の user_data 経路 (`Encoder::encode` の第 3 引数、 `EncodedFrame<T>::user_data()` で callback 側から取得可能) を使えば hisui 側で保持する必要がなくなる。

nvcodec decoder (`src/decoder/nvcodec.rs`) は既に user_data 経路を使っており (`self.inner.decode(&data, frame.to_stripped())`)、 encoder 側も対称化することで:

- `SharedInputQueue` 型 alias と関連コード (実測 ~30-40 行) を削除できる
- Mutex / VecDeque への hisui 側依存を減らせる (シンプル化)
- Ok の順序保証を hisui 側の queue から crate 内部の FIFO (`shiguredo_nvcodec-2026.2.0/src/encode.rs:1554, 1683-1689` の `pending_user_data` FIFO) に一本化できる
- issues/0085 で追加した「callback Err 分岐で `pop_front` を呼ぶ順序保証 contract」も削除できる (crate 側で drain Err 時に `pending_user_data.clear()` する挙動に委譲するため)

依存: issues/0085 (in-flight bp) は develop に merge 済み (`Merge pull request #320`)。 実装 PR は develop の現行状態から作業する。

## 優先度根拠

Low。 リファクタリングで新機能・バグ修正ではない。

- Ok 経路は挙動不変、 Err 経路も observable な差はない (§設計方針 §Err 経路の意味論変化 参照)
- decoder と encoder の対称化で読み手にとっての mental model が揃う
- §目的 で挙げた効果 (LOC 削減、 Mutex / VecDeque 依存削減、 順序保証の crate 一本化) が保守性向上として活きる
- 緊急性なし

## 現状

### hisui 側 (`src/encoder/nvcodec.rs`)

- `type SharedInputQueue = Arc<Mutex<VecDeque<VideoFrame>>>;` (`:15`)
- `NvcodecEncoder::input_queue: SharedInputQueue` フィールド (`:39`)
- `NvcodecEncoder::inner: shiguredo_nvcodec::Encoder<FnEncodeHandler<(), Error>>` (`:33-36`) の user_data 型が `()`
- `encode()` 内 `input_queue.lock().expect("nvcodec input queue lock poisoned").push_back(video_frame.to_stripped())` (`:345-351`) で metadata を保持
- callback Ok 分岐で `input_queue.lock().expect("...").pop_front()` (`:70-75`) して metadata 復元、 None なら `"encoded frame produced without input frame"` を emit_err (`:76-81`)
- callback Err 分岐でも `input_queue.lock().expect("...").pop_front()` (`:132-142` の Err arm 全体) を実行 (0085 で追加した順序保証 contract)
- 順序保証コメント (`:342-344`) で「Mutex 排他 + VecDeque FIFO + shiguredo_nvcodec 内部ワーカーの FIFO 処理」を説明
- `use std::collections::VecDeque;` (`:1`)、 `use std::sync::{Arc, Mutex, OnceLock};` (`:2`)

### decoder 側 (`src/decoder/nvcodec.rs`) - 対称化の参考

- `NvcodecDecoder::inner: shiguredo_nvcodec::Decoder<FnDecodeHandler<VideoFrame, Error>>` (`:12-14`) の user_data 型が `VideoFrame`
- `decode()` 内 `self.inner.decode(&data, frame.to_stripped())` (`:224`) として user_data を crate に渡す
- callback で `decoded.into_parts()` (`src/decoder/nvcodec.rs:83`) から user_data (input_frame) を復元

### crate `shiguredo_nvcodec 2026.2.0` の該当 API

- `EncodeHandler::UserData: Send + 'static` (`encode.rs:1159`) - user_data 型の trait bound
- `Encoder<H>::encode(&self, data: &[u8], options: &EncodeOptions, user_data: H::UserData) -> Result<(), shiguredo_nvcodec::Error>` (`encode.rs:1288-1301`)
- `EncodedFrame<T>::user_data(&self) -> &T` (`encode.rs:1447-1449`) / `into_parts(self) -> (Vec<u8>, T)` (`encode.rs:1451-1454`)
- crate 内部 worker が `Job::Encode` 受信時に `pending_user_data.push_back(user_data)` (`encode.rs:1584`) で FIFO 保持
- Ok drain 完了時: `pending_user_data.pop_front()` → `Some(user_data)` を `EncodedFrame` に載せて callback 発火 (`encode.rs:1683-1689`)
- Ok drain 完了時 (fallback): `pending_user_data.pop_front()` → `None` → `"consume_drain_result() failed: missing user data"` (Display 経由の実書式) を Err で callback 発火 (`encode.rs:1690-1696`)
- **drain Err 時**: `pending_user_data.clear()` で **pending 全件を一括 drop**、 その上で Err を callback 発火 (`encode.rs:1698-1703`)

## 設計方針

### user_data 経由への切り替え

- `NvcodecEncoder::inner` の型を `shiguredo_nvcodec::Encoder<shiguredo_nvcodec::FnEncodeHandler<VideoFrame, shiguredo_nvcodec::Error>>` に変更 (user_data 型を `()` → `VideoFrame` に)
- `NvcodecEncoder::input_queue: SharedInputQueue` フィールドを削除
- `SharedInputQueue` 型 alias 削除

### user_data payload の選定

`VideoFrame` を user_data として渡す (decoder との対称)。 callback で参照するのは `input_frame.size` と `input_frame.timestamp` のみだが、 `to_stripped()` は `data: Vec::new()` / `sample_entry: None` で軽量 clone を作るため per-frame オーバーヘッドは小さい。 `(Duration, Option<VideoFrameSize>)` タプル化する選択肢もあるが、 decoder の `VideoFrame::new_i420(input_frame, ...)` パターンとの視覚的整合を優先する。

`VideoFrame` が `Send + 'static` を満たすことは decoder 側の user_data 経路で既に実績あり (`VideoFrame` の全フィールドは `Vec<u8> / VideoFormat / bool / Option<VideoFrameSize> / Duration / Option<SharedSampleEntry>` で、 `SharedSampleEntry = Arc<SampleEntry>` は `Send + 'static`)。

### build_handler の変更

- `input_queue: SharedInputQueue` パラメータを削除
- 返り値型を `FnEncodeHandler<VideoFrame, shiguredo_nvcodec::Error>` に変更
- callback の Ok 分岐: `encoded_frame.picture_type()` でキーフレーム判定を先に行う (into_parts で consume されるため)、 続けて `encoded_frame.into_parts()` で `(data, input_frame)` を分岐の前で 1 回だけ取り出す。 現状は AV1 分岐だけ `into_parts()`、 H.264 / H.265 分岐は `encoded_frame.data()` で借用する非対称構造だが、 refactor 後は全分岐で input_frame を使うので分岐前で consume して data を local に保持する形になる:

    ```rust
    // 骨組み: FnEncodeHandler の閉包は `-> ()` のため `?` は使えない。
    // Err arm は現状 (`nvcodec.rs:114-121`) と同じ `match ... { Err(e) => { sink.emit_err(e); return; } }` で展開する。
    let keyframe = matches!(
        encoded_frame.picture_type(),
        shiguredo_nvcodec::PictureType::I | shiguredo_nvcodec::PictureType::Idr,
    );
    // AV1 分岐で Sequence Header OBU を先頭に付与するため `mut` で受ける。
    // input_frame は後段の `VideoFrame` 復元で `input_frame.size` / `input_frame.timestamp` として使う。
    let (mut data, input_frame) = encoded_frame.into_parts();
    let frame_data = if encoded_format == VideoFormat::Av1 {
        // AV1 分岐: has_sequence_header 判定 + Sequence Header OBU 前置 (現状 `nvcodec.rs:91-112` を踏襲)。
        data
    } else {
        match convert_annexb_to_mp4(&data) {
            Ok(d) => d,
            Err(e) => {
                sink.emit_err(e);
                return;
            }
        }
    };
    ```

- callback の Err 分岐: `input_queue.pop_front()` 呼び出しを削除 (crate 側の `pending_user_data.clear()` に委譲)。 実装後の Err 分岐は現状 `nvcodec.rs:132-142` の Mutex lock + pop_front ブロックを削って `sink.emit_err(crate::Error::new(format!("nvcodec encode error: {err}")))` の 1 行だけ残す形になる
- 「encoded frame produced without input frame」エラーパス (`:76-81`) は削除 (user_data 型が `Option` ではないため必ず存在する)。 同等 fallback は crate 側の `"consume_drain_result() failed: missing user data"` (`encode.rs:1690-1696`、 Display 経由の実書式) に温存される (§Err 経路の意味論変化 参照)
- `build_handler` の docstring (`nvcodec.rs:44-58`) を「callback スレッドで input_queue から pop」から「callback スレッドで `EncodedFrame::into_parts` から user_data を取り出して metadata (timestamp / size) 復元」に更新する

### encode() の変更

- `input_queue.lock().push_back(video_frame.to_stripped())` (`:345-351`) を削除
- `self.inner.encode(&nv12_data, &encode_options, ())` (`:360`) を `self.inner.encode(&nv12_data, &encode_options, video_frame.to_stripped())` に変更
- 順序保証コメント (`:342-344`) を削除 (crate 内部 `pending_user_data` FIFO に委譲する旨は §現状 で明示済みなので、 コード側では冗長)

### build_encoder の変更

- `let input_queue: SharedInputQueue = Arc::new(Mutex::new(VecDeque::new()));` (`:164`) を削除
- `build_handler` 呼び出しから `input_queue.clone()` (`:168`) を削除
- `Self` 初期化 (`:179-184`) から `input_queue` フィールドを削除
- `NvcodecEncoder` の `#[derive(Debug)]` (`:32`) はフィールド削除で自動追従するため手動対応不要

### HandlerContext / OnceLock 遅延スロットは触らない

`HandlerContext` (`:22-25`) と `HandlerContextSlot = Arc<OnceLock<HandlerContext>>` (`:30`) は「Encoder::new の handler consume 問題を解決するための遅延スロット」で、 codec ごとに 1 個の共有オブジェクト (`sample_entry` / `av1_sequence_header`) を保持する用途。 これを per-frame user_data に統合すると毎フレームで `SharedSampleEntry = Arc<SampleEntry>` の clone コスト (Arc 増減) が発生するため、 本 issue のスコープでは触らず OnceLock 経路を維持する。 `build_handler` のクロージャは `context_slot` の move キャプチャを引き続き行う。

### import 整理

`use std::collections::VecDeque;` (`:1`) を削除。 `use std::sync::{Arc, Mutex, OnceLock};` (`:2`) は `Mutex` を削除して `use std::sync::{Arc, OnceLock};` に減らす (`Arc` と `OnceLock` は `HandlerContextSlot` 用に保持)。

### Err 経路の意味論変化

crate 側の drain Err 挙動 (`encode.rs:1698-1703` の `pending_user_data.clear()`) 自体は refactor 前後で不変だが、 hisui 側で user_data の所有権が hisui → crate に移動することで内部実装差が生じる:

- refactor 前 (現状 hisui): callback Err 分岐で `input_queue.pop_front()` を 1 件だけ実行 → hisui 側 queue の残 pending は保持され、 crate 側の `pending_user_data.clear()` が非同期に走る間 hisui 側 queue と一時的に乖離する (0085 で対称化した順序保証はこの乖離を最小化する契約)
- refactor 後 (crate 委譲): 全ての pending は crate 側 `pending_user_data` に集約。 drain Err で `pending_user_data.clear()` により全 pending が一括 drop。 後続に Ok drain が到着すると crate 内 fallback `pop_front → None` 経路 (`encode.rs:1690-1696`) で `Error::new_custom("consume_drain_result", "missing user data")` が Err callback として発火する (Display 経由の実書式は `"consume_drain_result() failed: missing user data"`)

**実運用への影響評価**:

- drain Err の発生源は `nvEncLockBitstream` 失敗等の hardware / driver 起因の稀な path。 通常運用では発生しない
- `VideoEncoder::run` (`src/encoder.rs:784`) の output 分岐 (`:850-869`) は 1 件目の Err を受けた時点で `let frame = result?;` で早期 return し、 local `rx` が drop される
- 以降の crate worker からの callback Err (fallback path 含む) は `OutputSink::emit_err` (`src/encoder.rs:411`) 経由で `tx.send` されるが、 `OutputSink` docstring (`:380-385`) の「rx 閉鎖時は静かに破棄する」契約により silently discarded される。 `total_output_metric` も inc されない
- したがってユーザから見た挙動は不変


## 完了条件

### コード変更

§設計方針 各サブセクション通りの書き換えが完了していること。 grep 検証と cargo で機械確認できない意味論・不変事項は以下:

- callback Ok 分岐で `encoded_frame.picture_type()` → `encoded_frame.into_parts()` の順で処理する (into_parts で consume されるため picture_type が先)
- callback Ok の `Some(input_frame) else { emit_err(...) }` フォールバックが削除されている (crate 側 fallback に委譲)
- callback Err に `pop_front` 呼び出しがない
- `HandlerContext` / `HandlerContextSlot` / `OnceLock` 経路は不変
- `NvcodecEncoder::new_h264` / `new_h265` / `new_av1` / `encode` / `finish` / `request_keyframe` / `codec` の pub シグネチャは不変

### grep 検証

- `rg 'SharedInputQueue' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'input_queue' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'Arc<Mutex<VecDeque' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'std::collections::VecDeque' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'sync::.*Mutex' src/encoder/nvcodec.rs` の hit が **0 件** (`use std::sync::{...}` から Mutex が消えている)
- `rg 'encoded frame produced without input frame' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg 'nvcodec input queue lock poisoned' src/encoder/nvcodec.rs` の hit が **0 件**
- `rg '順序保証' src/encoder/nvcodec.rs` の hit が **0 件**

### テスト

- 他の同層 encoder (libvpx / openh264 / svt_av1 / video_toolbox) には `make_encoder_sink` (`src/encoder/test_helpers.rs`) 経由の software encoder 実 encode unit test が存在する (`src/encoder/libvpx.rs::mod tests`, `src/encoder/openh264.rs::mod tests`, `src/encoder/svt_av1.rs::mod tests`, `src/encoder/video_toolbox.rs::mod tests`)
- nvcodec は encode 経路全体が CUDA / GPU 依存で、 crate 内部 `EncodedFrame` の直接構築 API も pub でないため、 make_encoder_sink 相当の unit test 経路は書けない (nvcodec は CI の `test-nvidia-video-codec` job で `tests/` 配下の e2e に委譲する構造)。 本 issue は crate の worker スレッド挙動をモックする unit test も作らない (CLAUDE.md 「モックやスタブは絶対に利用しないこと」規約準拠)
- Ok 経路の回帰検出: `tests/e2e.rs::simple_single_source_h264` / `simple_single_source_h265` (`#[cfg(any(feature = "nvcodec", target_os = "macos"))]` 配下、 nvcodec feature 有効時に nvcodec H.264 / H.265 encoder が選ばれる) と、 `simple_single_source_av1` (cfg 属性なし、 default は svt_av1 経路。 `--features nvcodec` build で encoder 選択が nvcodec に切り替わる場合に本 refactor 経路を通る) が、 user_data を通じた timestamp / size の per-frame 復元が正しく行われれば既存の frame metadata assert を通る (回帰したら metadata 不整合で fail する)
- Err 経路の差異は §Err 経路の意味論変化 で observable でないと結論しているため e2e で拾わないことは妥当

### cargo

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo check --workspace --features nvcodec`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`
- `cargo test --features nvcodec -p hisui -- --test-threads=1` (`.github/workflows/ci.yml` の `test-nvidia-video-codec` job で CI 自動実行される。 本 issue は unit test を追加しないため CI の実行内容は不変で、 `tests/` 配下の e2e は CI 側でカバーされる)

すべて通る。

## 解決方法

polish 完了後に追記する。

## CHANGES.md について

**記載不要**。 Ok 経路の挙動は不変。 Err 経路は crate 側の `pending_user_data.clear()` に委譲する形で意味論が微妙に異なる (§設計方針 §Err 経路の意味論変化 参照) が、 `VideoEncoder::run` が 1 件目の Err で早期 return するため hisui pipeline 側からは observable な差異が発生しない。

## 関連

- issues/0085 (`feature/refactor-encoder-inflight-backpressure`): 依存先。 実装 PR は develop に merge 済み。 issue ファイルは open のまま (closed 移動待ち)。 0085 で追加した「callback Err 分岐 `pop_front`」と「順序保証コメント」は本 issue で自然に削除される
- issues/0087 (`feature/add-realtime-encoder-param-override`、 open): 独立。 順不同で着手可 (0087 は `src/encoder.rs`、 本 issue は `src/encoder/nvcodec.rs` を触るためコード的に衝突しない)
- closed/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`、 2026-07-21 closed): 0085 の前身。 nvcodec encoder 内部構造の直前史として参照
- `src/decoder/nvcodec.rs`: 対称化の参考実装 (既に `FnDecodeHandler<VideoFrame, Error>` 経由で user_data を使用)
- closed/0057 §3 分割表: 本 issue は encoder inner 構造の内部 refactor で 0057 §3 の分割単位に対応しないため、 §3 表への追記は不要
