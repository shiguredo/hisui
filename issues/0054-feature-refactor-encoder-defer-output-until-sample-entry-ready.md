# openh264 と VideoToolbox H.264 経路で sample_entry 未確定間の出力フレームを保留する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-encoder-defer-output-until-sample-entry-ready
- Polished: 2026-06-23

## 目的

issue 0051 で writer 入口の sample_entry fallback 補完経路を全削除した結果、エンコーダ側に残る「最初の出力フレームが必ず keyframe で SPS / PPS が揃う」という暗黙前提のフェイルセーフが失われた。本 issue では openh264 全体と VideoToolbox の H.264 経路で「sample_entry が確定するまで出力フレームを内部退避し、確定後に保留分を一括 push する」設計に変更し、入力側不変条件（圧縮フレームには常に `sample_entry: Some` を付与）を実装レベルで堅牢化する。

外部 API 変更を伴わない内部実装の堅牢化リファクタとして `feature/refactor-` を採用する。

## 優先度根拠

Low。openh264 / VTCompressionSession の通常動作では「最初の出力フレームが必ず keyframe」となり、現状の運用で破綻シナリオは観測されていない。ただしこれは API レベルの保証ではなく暗黙の運用前提であり、以下のような将来シナリオで前提が崩れる可能性がある:

- VTCompressionSession の B フレーム並べ替えで非 keyframe が先に出力される
- openh264 が SPS のみのフレームを早期に出力する
- macOS / openh264 ライブラリの更新で keyframe 出力タイミングが変わる

前提が崩れた場合の writer 側挙動は `docs/internals/sample_entry_invariant.md` の「writer 側の前提」節で整理済みで、本 issue は encoder 側で対処する。対の関係にある writer 側の fail-safety 補強は issue 0055 が扱う。

## 現状

- `src/encoder/openh264.rs::Openh264Encoder::encode`: SPS / PPS が空のフレームでは `last_sample_entry` を `None` のまま保持しつつ、出力スロット `encoded: Option<VideoFrame>` に `sample_entry: None` の `VideoFrame` を入れ、`next_encoded_frame` で取り出される。出力スロットは `VecDeque` ではなく `Option` の単発。
- `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded` の H.264 経路: `frame.sps_list.is_empty() || frame.pps_list.is_empty()` の場合 `self.sample_entry` を確定せず、`output_queue: VecDeque<VideoFrame>` に `sample_entry: None` の `VideoFrame` を push する。
- 同 `VideoToolboxEncoder` の H.265 経路: `src/video/h265.rs::h265_sample_entry` が空 VPS / SPS / PPS リストでも常に `Ok(SampleEntry::Hvc1(..))` を返す実装のため、初回フレームから無条件で `self.sample_entry` が確定する。結果として「`sample_entry: None` の `VideoFrame` を `output_queue` に積む経路」が存在しない。本 issue の対象は openh264 全体 + VideoToolbox H.264 経路のみ。H.265 経路の「空 NALU 配列で hvcC を作ってしまう」点は別問題（本 issue の不変条件 = `sample_entry: Some` の範囲には抵触しない）。
- `docs/internals/sample_entry_invariant.md` の「確立できない場合の扱い」節で、本経路が「API 保証ではない暗黙の運用前提」に依存している旨を明示し、実装レベルでの堅牢化は本 issue として整理してある。

## 設計方針

上限超過 / `finish` 残置時の `Err` メッセージ文言は設計方針 3 に集約する。設計方針 1 / 2 では `Err` を返す条件のみを記述する。

### 1. openh264 の出力経路改修

`src/encoder/openh264.rs::Openh264Encoder`:

1. 出力スロットを `encoded: Option<VideoFrame>` から `output_queue: VecDeque<VideoFrame>` に置き換える。`next_encoded_frame` は `self.output_queue.pop_front()` を返す。内部エンコーダの 1 回の `encode` 呼び出しで返るフレームは最大 1 個のままで、`VecDeque` 化は「保留中の複数フレームを順序保ったまま保持する」目的の構造変更（同時保持される複数フレームを許す API への一般化）。
2. 新規フィールド `pending_output: VecDeque<VideoFrame>` を追加する。
3. 現状の SPS / PPS 検出位置（既存 `encode` 内で `last_sample_entry` を更新している箇所）はそのまま維持し、その時点で `last_sample_entry` を更新する。
4. `VideoFrame` を組み立てるときは `sample_entry: self.last_sample_entry.clone()` で構築する。`last_sample_entry` が `Some` であればそのフレームは即 `output_queue.push_back`、`None` であれば `pending_output.push_back` で退避する。確定タイミングフレーム自身は前段 3 の更新により `last_sample_entry` が `Some` になった後に判定されるため、`pending_output` を経由せず直接 `output_queue` に積まれる。
5. `last_sample_entry` が初めて `Some` に遷移した同一 `encode` 呼び出し内で、まず `pending_output.drain(..)` を回し、各退避フレームの `VideoFrame.sample_entry`（`pub` フィールド）を `Some(self.last_sample_entry.clone().expect("ここでは Some が保証されている"))` で書き換えて `output_queue.push_back` する。その後で確定タイミングフレーム自身を `output_queue.push_back` する。pending_output の各エントリは内部エンコーダの出力順で並ぶため、確定タイミングフレームより前に出力されたフレームのみが含まれる。本層では出力順をそのまま維持し再順序付けは行わない。Hisui のパイプラインは writer 側で `composition_time_offset: None` ハードコード（`src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs`）かつ HLS / DASH の `reorder_payload_by_track` は mdat 内バイト配置のみで PTS 並べ替えを行わない構造のため、encoder 出力は「DTS = PTS、出力順 = 入力 PTS 順」を満たす必要がある。openh264 は B フレームを使わない前提（`shiguredo_openh264` ライブラリの設計コメント参照）、VideoToolbox は `allow_frame_reordering: false` を固定値として使用する前提（`src/sora/recording_encoder_video_toolbox_params.rs` 参照）でこれを満たす。B フレーム並べ替えを伴う構成は本不変条件の対象外とし、必要になった時点で writer 側に `composition_time_offset` 対応を入れる別 issue として扱う。
6. `pending_output.len()` が上限 `MAX_PENDING_OUTPUT_FRAMES` を超えた時点で `crate::Error::new(...)` を返す。
7. 上限超過 `Err` を返した後は `pending_output` を `clear()` し、それ以降の `encode` / `finish` は通常通り受け付ける（エンコーダ再構築は不要）。
8. `finish` で `pending_output` が空でない場合は `crate::Error::new(...)` を返す。
9. 上限超過 `Err` **以外** のエンコーダ内 Err（`h264_sample_entry_from_sps_pps_lists` 失敗、`inner.encode` 失敗等）は、エンコーダ全体が使用不能になる前提で `pending_output` の clear は行わず、そのまま `Err` を上位に返す。上位はエンコーダインスタンスを破棄する。
10. `force_idr_pending`（`request_keyframe` 経路）は既存実装どおり、is_keyframe 出力で false にリセットされるまでフラグが立ち続ける挙動を維持し、保留バッファとは独立に動作する。

### 2. VideoToolbox の出力経路改修

`src/encoder/video_toolbox.rs::VideoToolboxEncoder`:

1. 新規フィールド `pending_output: VecDeque<VideoFrame>` を追加する。既存の `output_queue: VecDeque<VideoFrame>` はそのまま残す。
2. `handle_encoded` の `while let Some(frame) = self.inner.next_frame()?` ループの各反復で、まず現状の確定処理（`self.sample_entry.is_none()` ガード内で `sample_entry_opt` を計算し、`Some` なら `self.sample_entry` に書き込む）を行い、その後 `VideoFrame` を組み立てて設計方針 1 と同じ規約で振り分ける:
   - `self.sample_entry` が `Some` であれば `VideoFrame` を `sample_entry: self.sample_entry.clone()` で構築し `output_queue.push_back`
   - `self.sample_entry` が `None` のままであれば `VideoFrame` を `sample_entry: None` で構築し `pending_output.push_back`
   - 確定タイミングとなったループ反復（`self.sample_entry` が `None` から `Some` に遷移した反復）では、設計方針 1-5 と同じ順序で「まず `pending_output.drain(..)` で退避フレームを `output_queue` に流し、その後に当該反復のフレーム自身を `output_queue.push_back` する」
3. 確定処理（`sample_entry_opt` の計算）は format によらず現状の H.264 / H.265 分岐をそのまま使う。退避判定も format によらず `self.sample_entry.is_none()` のみで行う。H.265 経路は `h265_sample_entry` が空入力でも `Ok` を返す実装上、初回反復で必ず確定するため `pending_output` は仕様上常に空のまま運用される（`finish` 時の `is_empty` チェックも常に通る）。
4. 上限超過 `Err`、上限超過後の `clear` 再開、`finish` 時 pending 残置 `Err`、上限超過以外の Err でのエンコーダ使用不能扱い、状態遷移規約は openh264 と同じ。
5. `keyframe_request_pending`（`request_keyframe` 経路）は既存実装どおり、次回 `encode()` 呼び出し時に内部エンコーダに渡された直後 false にリセットされる挙動を維持する。openh264 と保持期間の挙動が非対称になるが、`pending_output` の有無で挙動を変えない意味で「同じ規約」として扱う。

### 3. バッファ上限値とエラーメッセージ

`const MAX_PENDING_OUTPUT_FRAMES: usize = 64;` を `src/encoder/openh264.rs` / `src/encoder/video_toolbox.rs` 双方で個別に定義する。共通定数化はしない: openh264 と VideoToolbox は GOP 構造と保留要因が異なり、上限値を独立に調整できる方が将来の挙動変化に追随しやすいため、別定数として保持する。

上限値の根拠: 通常動作では 0 〜 1 フレームで sample_entry が確定する。VTCompressionSession の B フレーム並べ替えウィンドウは概ね GOP 内の数フレーム規模、openh264 も同等で、健全状態でも到達し得る最大値は数十フレーム程度と見積もる。64 はこれに余裕を持たせた上での「異常状態の検知」上限として固定する。

エラーメッセージは `format!` の位置引数で組み立てる（`{IDENT}` のキャプチャ構文は const に対しては使えないため）:

- 上限超過 (openh264): `format!("openh264 encoder pending output overflow before sample_entry is established (limit={})", MAX_PENDING_OUTPUT_FRAMES)`
- 上限超過 (video_toolbox): `format!("video_toolbox encoder pending output overflow before sample_entry is established (limit={})", MAX_PENDING_OUTPUT_FRAMES)`
- `finish` 時 pending 残置 (openh264): `format!("openh264 encoder finished without establishing sample_entry; {} frames discarded", self.pending_output.len())`
- `finish` 時 pending 残置 (video_toolbox): `format!("video_toolbox encoder finished without establishing sample_entry; {} frames discarded", self.pending_output.len())`

CLAUDE.md「ログメッセージは全て英語にすること」に従い英語で書く。

### 4. 上位パイプラインへの影響評価

`src/encoder.rs::VideoEncoder::drain_encoded_frames` は内側エンコーダ出力を `while let Some(encoded) = inner.next_encoded_frame()` で吸い出して `VideoEncoder::encoded`（出力 VecDeque）に積む。これとは別経路で `drain_video_encoder_output` のループから呼ばれる `VideoEncoder::poll_output` が `self.encoded` 空かつ未 EOS の時点で `EncoderRunOutput::Pending`（`src/encoder.rs` 内 private enum）を返し、上位の `run` ループが次の入力メッセージ受信を待つ構造。本改修により sample_entry 確定までの間 `next_encoded_frame` が連続して `None` を返すが、上位パイプラインは `Pending` 経路で破綻しない。

上限超過 `Err` / `finish` 時 pending 残置 `Err` / その他の内部 `Err` は `encode` / `finish` の返り値経由で `drain_encoded_frames` に到達し、上位の Err 取扱いに従って fail-fast 停止する（writer 入口 fallback 削除後の `MissingSampleEntry` Err と同じ位置づけ）。

### 5. テスト追加

`src/encoder/openh264.rs` 既存の `#[cfg(test)] mod tests` に追記し、`src/encoder/video_toolbox.rs` には同形式（`#[cfg(test)] mod tests`）で新設する（既存 `src/encoder/*.rs` の `mod tests` 配置に倣う）。

テストアクセス: 既存 `assert_carries_latest_sample_entry`（`openh264.rs` 内）が `encoder.last_sample_entry` フィールドへ同モジュール内テストから直接アクセスするのと同流儀で、各テスト内で `assert!(encoder.pending_output.is_empty())` のように `encoder.pending_output` を直接観測する。test-only helper や専用アクセサは追加しない。

追加するテストの観点:

- 既存テスト（`openh264_sets_sample_entry_on_every_output_frame` / `openh264_sets_sample_entry_after_keyframe_request`）が引き続き通り、初回 keyframe 出力で sample_entry が確定し、それ以降の `output_queue` から `sample_entry: None` が出ないこと
- 退避状態の観測: 実エンコーダで「最初の出力が keyframe」となる通常動作下では退避バッファは即フラッシュされるため `encoder.pending_output.len() > 0` を直接観測するテストは書けない。代わりに sample_entry 確定後に `encoder.pending_output.is_empty()` が成り立つこと、および `next_encoded_frame()` が返す全フレームに `sample_entry: Some(..)` が載ること（既存テスト相当）で間接的に保証する
- 「最初の keyframe を一度も出させない / 上限まで非 keyframe を積ませる」状態をモック禁止下で実エンコーダから作る現実的手段が無いため、`finish` 時 pending 残置 `Err` / 上限超過 `Err` の単体テストは本 issue では追加せず、完了条件からも外す

テスト実行環境:

- openh264 系テストは既存と同様に `OPENH264_PATH` 未設定環境ではスキップ
- VideoToolbox 系テストは `cargo test --features video_toolbox` で実行（macOS）

### 6. ドキュメント更新

`docs/internals/sample_entry_invariant.md` の以下を書き換える:

- エンコーダ責務分担表（44-46 行目相当）の openh264 / VideoToolbox 行の「サンプルエントリー確立タイミング」列を「最初の keyframe の SPS / PPS で確定するまで内部退避、確定後に保留フレームを一括 push および以降全フレームへ伝播」に書き換える
- 「確立できない場合の扱い」節（53-61 行目相当）の最終段落を以下に置き換える:

```
エンコーダ側で「最初の keyframe より前に出力が出る」可能性を持つもの（openh264 / VideoToolbox H.264 経路）は、sample_entry が確定するまで出力フレームを内部退避し、SPS / PPS が揃ったタイミングで保留分を一括 push する。これにより writer 入口で `sample_entry: None` の圧縮フレームを観測することは仕様上不可能となる。退避フレーム数が上限を超えた場合、および sample_entry を確立せずに `finish` された場合はエンコーダ Err を返す。

なお `VideoToolboxEncoder` の H.265 経路は `h265_sample_entry` が空 VPS / SPS / PPS リストでも `Ok` を返す実装のため、初回フレームから無条件で sample_entry が確定する。空 NALU 配列で hvcC を作ってしまう挙動の妥当性は別途検討する余地があるが、本不変条件としては `sample_entry: Some` が常に立つことだけが保証されればよい。
```

### 7. CHANGES.md

`## develop` へは記載しない。本改修は writer 入口 fallback 削除（issue 0051）と同じく未リリース期間の内部実装堅牢化で、shiguredo-changelog の「派生元ブランチとの最終的な差分のみを記載すること」に該当しない。新規 `Err` 条件は理論上観測可能だが、入力側不変条件が確立している環境では実運用上発火しない（writer 入口 fallback と同じ位置づけ）。

## スコープ

含むもの:

- `src/encoder/openh264.rs` の出力経路改修（`encoded` の `output_queue: VecDeque<VideoFrame>` 化、`pending_output` 追加、振り分けロジック、`finish` Err、上限 Err、状態遷移規約）と単体テスト追加
- `src/encoder/video_toolbox.rs` の出力経路改修（`pending_output` 追加、振り分けロジック、`finish` Err、上限 Err、状態遷移規約）と単体テスト追加
- `docs/internals/sample_entry_invariant.md` の表・「確立できない場合の扱い」節の書き換え
- `src/encoder.rs::VideoEncoder::drain_encoded_frames` / `poll_output` の再確認（コード変更を伴わない可能性が高い）

含まないもの:

- NVENC / svt_av1 / libvpx / fdk-aac / AudioToolbox / Opus 経路（これらはコンストラクタで SDK 提供のシーケンスヘッダ取得関数相当の API を使って sample_entry を確定する設計のため対象外）
- VideoToolbox H.265 経路の「空 VPS / SPS / PPS で `Hvc1Box` を作ってしまう」問題（本 issue の不変条件 = `sample_entry: Some` の範囲には抵触しない）
- writer 入口の fallback 復活（issue 0051 で確立した「責任の所在を入力側に集約する」方針を維持する）
- writer 側 `composition_time_offset` 対応（B フレーム並べ替えに伴う DTS / PTS 分離は本 issue の対象外）
- 本 issue で追加するコード / コメント / テスト / docstring に `issue NNNN` 形式の参照を含めない（shiguredo-issues 規約。新規 docstring / コメントの「なぜ」は理由そのものを書く）

## 完了条件

- openh264 と VideoToolbox H.264 経路の `next_encoded_frame()` が返す全フレームに `sample_entry: Some(..)` が載ることをテストで網羅的に観測する（既存テスト相当）
- sample_entry 確定後に `encoder.pending_output.is_empty()` が成り立つことをテストで観測する
- 既存テスト（`openh264_sets_sample_entry_on_every_output_frame` / `openh264_sets_sample_entry_after_keyframe_request` 等）が引き続き通る
- `next_encoded_frame` が sample_entry 確定までの間連続 `None` を返しても上位パイプライン（`VideoEncoder::drain_encoded_frames` / `poll_output` / `EncoderRunOutput::Pending`）が破綻しないことをコード読みで確認する
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通る（`video_toolbox` feature を含む）
- `docs/internals/sample_entry_invariant.md` の記述が新実装と整合している

## 関連

- closed/0051（writer 入口 fallback 削除。本 issue の前提）
- 0055（HlsWriter MpegTs 経路の Err 化。本 issue と性質が一対）
- closed/0017 / closed/0027（エンコーダの sample_entry 全フレーム付与）
- `docs/internals/sample_entry_invariant.md`
