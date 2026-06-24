# openh264 と VideoToolbox エンコーダで sample_entry 未確定時の出力フレームを Err にする

- Priority: Low
- Created: 2026-06-23
- Completed: 2026-06-24
- Model: Claude Opus 4.7
- Branch: feature/refactor-encoder-defer-output-until-sample-entry-ready
- Polished: 2026-06-23

## 目的

issue 0051 で writer 入口の sample_entry fallback 補完経路を全削除した結果、エンコーダ側に残る「最初の出力フレームが必ず keyframe で SPS / PPS (H.265 は VPS も) が揃う」という暗黙前提のフェイルセーフが失われた。本 issue では openh264 全体と VideoToolbox の H.264 / H.265 両経路で「sample_entry が確定する前に出力フレームを組み立てようとしたら即 Err を返す」設計に変更し、入力側不変条件（圧縮フレームには常に **有効な** sample_entry を付与する）を実装レベルで fail-fast 化する。

退避バッファによる救済設計は採用しない。理由: openh264 / VTCompressionSession の通常動作で「最初の出力が必ず keyframe」が成立するため、非 keyframe 先行出力が起きるのはエンコーダの挙動が暗黙前提から外れた異常状態であり、救済より早期検知の方が運用上有益。退避設計はコード量・テスト負荷を増やすが、救済できるシナリオが現実的に存在しない死活経路となるため避ける。

外部 API 変更を伴わない内部実装の堅牢化リファクタとして `feature/refactor-` を採用する。

## 優先度根拠

Low。openh264 / VTCompressionSession の通常動作では「最初の出力フレームが必ず keyframe」となり、現状の運用で破綻シナリオは観測されていない。ただしこれは API レベルの保証ではなく暗黙の運用前提であり、以下のような将来シナリオで前提が崩れる可能性がある:

- VTCompressionSession の B フレーム並べ替えで非 keyframe が先に出力される
- openh264 が SPS のみのフレームを早期に出力する
- macOS / openh264 ライブラリの更新で keyframe 出力タイミングが変わる

前提が崩れた場合の writer 側挙動は `docs/internals/sample_entry_invariant.md` の「writer 側の前提」節で整理済みで、本 issue は encoder 側で対処する。対の関係にある writer 側の fail-safety 補強は issue 0055 が扱う。

## 現状

- `src/encoder/openh264.rs::Openh264Encoder::encode`: SPS / PPS が空のフレームでは `last_sample_entry` を `None` のまま保持しつつ、出力スロット `encoded: Option<VideoFrame>` に `sample_entry: None` の `VideoFrame` を入れ、`next_encoded_frame` で取り出される。`sample_entry: None` を下流に渡す経路が存在する。
- `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded` の H.264 経路: `frame.sps_list.is_empty() || frame.pps_list.is_empty()` の場合 `self.sample_entry` を確定せず、`output_queue: VecDeque<VideoFrame>` に `sample_entry: None` の `VideoFrame` を push する。
- 同 `VideoToolboxEncoder` の H.265 経路: `src/video/h265.rs::h265_sample_entry` が空 VPS / SPS / PPS リストでも常に `Ok(SampleEntry::Hvc1(..))` を返す実装のため、空入力でも形式上の `sample_entry: Some` を構築できてしまう。型レベルの不変条件 (`Some` であること) は満たせるが、`hvcC` の `nalu_arrays` が空のままハードコード固定値で記入された壊れた `Hvc1Box` を下流に流す。意味的不変条件 (有効な sample_entry) は満たせない。
- `docs/internals/sample_entry_invariant.md` の「確立できない場合の扱い」節で、本経路が「API 保証ではない暗黙の運用前提」に依存している旨を明示し、実装レベルでの堅牢化は本 issue として整理してある。

## 設計方針

### 1. openh264 の出力経路改修

`src/encoder/openh264.rs::Openh264Encoder::encode` を以下に変更する:

1. 現状の SPS / PPS 検出位置（既存 `encode` 内で `last_sample_entry` を更新している箇所）はそのまま維持する。SPS / PPS が含まれるフレームでは `last_sample_entry` を `Some` に更新する。
2. 上記更新の **後** で `self.last_sample_entry.is_none()` を判定し、`None` のまま `VideoFrame` を組み立てようとするなら `crate::Error::new(...)` を返して fail-fast 停止する。エラーメッセージは `"openh264 encoder produced output before SPS/PPS established the sample_entry"`。
3. `last_sample_entry` が `Some` になっている場合は現状どおり `self.encoded = Some(VideoFrame { ..., sample_entry: self.last_sample_entry.clone() })` で組み立てる。
4. `next_encoded_frame` / `finish` / `request_keyframe` の挙動は現状維持。出力スロット `encoded: Option<VideoFrame>` は変更しない。

退避バッファ・上限超過 Err・finish 時 pending 残置 Err・状態遷移規約は導入しない。

### 2. VideoToolbox の出力経路改修

`src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded` を以下に変更する:

1. 現状の確定処理（`self.sample_entry.is_none()` ガード内で `sample_entry_opt` を計算）は維持する。
2. H.264 経路で `frame.sps_list.is_empty() || frame.pps_list.is_empty()` のときに `None` を返す現状の分岐を、`crate::Error::new(...)` を返して即 fail-fast 停止する分岐に置き換える。エラーメッセージは `"video_toolbox encoder produced H.264 output before SPS/PPS established the sample_entry"`。
3. H.265 経路で `frame.vps_list.is_empty() || frame.sps_list.is_empty() || frame.pps_list.is_empty()` のときに `crate::Error::new(...)` を返して即 fail-fast 停止する分岐を追加する。エラーメッセージは `"video_toolbox encoder produced H.265 output before VPS/SPS/PPS established the sample_entry"`。`h265_sample_entry` 自体には手を入れず、呼び出し側でガードする (RTSP / WebM など他経路への副作用を避ける)。
4. `output_queue` への push、`next_encoded_frame` / `finish` / `request_keyframe` の挙動は現状維持。退避バッファ・上限超過 Err・finish Err・状態遷移規約は導入しない。

### 3. 上位パイプラインへの影響評価

`src/encoder.rs::VideoEncoder::drain_encoded_frames` は内側エンコーダ出力を `while let Some(encoded) = inner.next_encoded_frame()` で吸い出して `VideoEncoder::encoded` に積み、`VideoEncoder::poll_output` が空のターンでは `EncoderRunOutput::Pending` を返す。本改修で追加する Err は `encode` / `handle_encoded` の返り値経由で `drain_encoded_frames` に到達し、上位の Err 取扱いに従って fail-fast 停止する（writer 入口 fallback 削除後の `MissingSampleEntry` Err と同じ位置づけ）。

### 4. テスト方針

既存テスト（`openh264_sets_sample_entry_on_every_output_frame` / `openh264_sets_sample_entry_after_keyframe_request` / `video_toolbox_h264_sets_sample_entry_on_every_output_frame` / `video_toolbox_h265_sets_sample_entry_on_every_output_frame`）が引き続き通ることで「通常動作では Err 経路に到達しないこと」を確認する。

CI 検証経路: openh264 系テストは `.github/workflows/ci.yml` の `test-openh264` ジョブで libopenh264 を実体配置して `OPENH264_PATH` を渡して走らせる。VideoToolbox 系テストは self-hosted macOS ARM64 で `cargo test --workspace` を回す `test-apple-toolbox` ジョブで担保する。env 未設定でローカル `cargo test` を実行した場合は openh264 系テストが `eprintln!` してスキップする実装で、これは libvpx / svt_av1 / VideoToolbox など他エンコーダ単体テストと同じ流儀。

新規 Err 経路の単体テストは追加しない。理由: モック禁止下で実エンコーダから「最初の出力が非 keyframe」となる状態を作る現実的手段が無いため、テストとして実装できない。test-only helper や設定変更は追加負債のため取らない。

### 5. ドキュメント更新

`docs/internals/sample_entry_invariant.md` の以下を書き換える:

- エンコーダ責務分担表（44-46 行目相当）の openh264 / VideoToolbox 行の「サンプルエントリー確立タイミング」列を、Err 化を反映した記述に書き換える
- 「確立できない場合の扱い」節（53-61 行目相当）の最終段落を、退避設計ではなく Err 化設計の記述に置き換える
- 不変条件の表現を「`sample_entry: Some`」(型レベル) から「**有効な** sample_entry」(意味レベル) に揃える

書き換え後の文言は実装結果に合わせて調整する。

### 6. CHANGES.md

`## develop` へは記載しない。本改修は writer 入口 fallback 削除（issue 0051）と同じく未リリース期間の内部実装堅牢化で、shiguredo-changelog の「派生元ブランチとの最終的な差分のみを記載すること」に該当しない。新規 `Err` 条件は理論上観測可能だが、入力側不変条件が確立している環境では実運用上発火しない。

## スコープ

含むもの:

- `src/encoder/openh264.rs::Openh264Encoder::encode` への Err 経路追加
- `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded` への Err 経路追加 (H.264 / H.265 両経路)
- `docs/internals/sample_entry_invariant.md` の表・「確立できない場合の扱い」節の書き換え
- `src/encoder/test_helpers.rs` 新設による `raw_i420_frame` の 4 エンコーダ間共通化 (本 PR のレビューで指摘された既存重複)

含まないもの:

- NVENC / svt_av1 / libvpx / fdk-aac / AudioToolbox / Opus 経路（コンストラクタで sample_entry が確定する設計のため対象外）
- `src/video/h265.rs::h265_sample_entry` 自体の空入力チェック追加（RTSP / WebM など他経路への副作用を避け、本 issue では呼び出し側でガードする）
- writer 入口の fallback 復活（issue 0051 で確立した「責任の所在を入力側に集約する」方針を維持する）
- writer 側 `composition_time_offset` 対応（B フレーム並べ替えに伴う DTS / PTS 分離は本 issue の対象外）
- 退避バッファによる救済設計（コード量・テスト負荷を増やすが救済シナリオが死活経路となるため不採用）
- 本 issue で追加するコード / コメント / テスト / docstring に `issue NNNN` 形式の参照を含めない（shiguredo-issues 規約。新規 docstring / コメントの「なぜ」は理由そのものを書く）

## 完了条件

- openh264 / VideoToolbox の全経路 (H.264 / H.265) で `sample_entry` が未確定のまま `VideoFrame` を組み立てようとした場合に `Err` を返すことが実装されている
- 既存テスト（`openh264_sets_sample_entry_on_every_output_frame` / `openh264_sets_sample_entry_after_keyframe_request` / `video_toolbox_h264_sets_sample_entry_on_every_output_frame` / `video_toolbox_h265_sets_sample_entry_on_every_output_frame`）が引き続き通る
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通る
- `docs/internals/sample_entry_invariant.md` の記述が新実装と整合している

## 関連

- closed/0051（writer 入口 fallback 削除。本 issue の前提）
- 0055（HlsWriter MpegTs 経路の Err 化。本 issue と性質が一対）
- closed/0027（エンコーダの sample_entry 全フレーム付与）
- `docs/internals/sample_entry_invariant.md`

## 解決方法

### sample_entry 未確定時の Err 化 (本体)

- `src/encoder/openh264.rs::Openh264Encoder::encode` で SPS / PPS 検出による `last_sample_entry` 更新の **後** に `self.last_sample_entry.is_none()` を判定し、未確定なら `crate::Error::new(...)` で fail-fast 停止する経路を追加した。
- `src/encoder/video_toolbox.rs::VideoToolboxEncoder::handle_encoded` の H.264 経路で SPS / PPS 空のときに同様の Err 経路を追加した。さらに H.265 経路でも `frame.vps_list` / `sps_list` / `pps_list` のいずれかが空なら Err を返すガードを追加した。`h265_sample_entry` 自体には手を入れず、呼び出し側でガードする方針 (RTSP / WebM など他の呼び出し経路への副作用を避ける)。
- 退避バッファ・上限超過 Err・finish 時 pending 残置 Err は採用しなかった (救済シナリオが死活経路となるため)。検討経緯は本 issue のコミット履歴に残る (A 設計 → B 設計の切り替え)。

### VideoToolbox の構成前提を実装で守る

- `src/encoder/video_toolbox.rs::VideoToolboxEncoder::new_h264` / `new_h265` で `config.allow_frame_reordering == true` ならコンストラクタ Err にした。B フレーム並べ替えで `input_queue` ペアリングが崩れる前提を実装レベルで守る。`shiguredo_video_toolbox` 側も `allow_frame_reordering: false` を前提として設計されている。

### Err メッセージの診断情報

- openh264 / video_toolbox の Err 文に入力 PTS と `is_keyframe` (または `frame.keyframe`) を含めた。異常状態の種類 (非 keyframe 先行か、keyframe なのに SPS / PPS なしか) を切り分けられる。openh264 では `is_keyframe` 判定を Err 直前に持ち上げた。

### テストヘルパ共通化

- `src/encoder/test_helpers.rs` を新設し、4 エンコーダ (openh264 / video_toolbox / libvpx / svt_av1) で重複していた `raw_i420_frame` を集約した。`#[cfg(test)] mod test_helpers;` を `src/encoder.rs` で宣言し、各エンコーダの `mod tests` から `use crate::encoder::test_helpers::raw_i420_frame;` で利用する。
- openh264 テストの `assert_carries_latest_sample_entry` ヘルパを 2 つのテスト関数の前にまとめてヘルパ群を集約した。

### ドキュメント更新

- `docs/internals/sample_entry_invariant.md` の不変条件節を「型レベル (`Some` であること) と意味レベル (中身が有効であること) の双方」を要求する形に書き換えた。空 NALU 配列で構築された壊れた `hvcC` は型レベルでは `Some` を満たすが意味的不変条件は満たさない、と明示した。
- エンコーダ責務分担表の openh264 / VideoToolbox 行を「最初の keyframe の SPS / PPS (H.265 は VPS も) から確定、確定前に出力が起きた場合はエンコーダ Err」に揃えた。
- 「確立できない場合の扱い」節を Err 化方針に書き換えた。

### CI 検証経路

- 本改修の通常動作検証は `.github/workflows/ci.yml` の `test-openh264` ジョブ (Ubuntu + libopenh264 を Cisco からダウンロードして `OPENH264_PATH` 設定) と `test-apple-toolbox` ジョブ (self-hosted macOS ARM64) で担保される旨を issue テスト方針節に注記した。

### CHANGES.md

`## develop` には記載しなかった。本改修は writer 入口 fallback 削除 (issue 0051) と同じく未リリース期間の内部実装堅牢化で、shiguredo-changelog の「派生元ブランチとの最終的な差分のみを記載すること」に該当しない。新規 `Err` 条件は理論上観測可能だが、入力側不変条件が確立している環境では実運用上発火しない。
