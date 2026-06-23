# openh264 / VideoToolbox エンコーダで sample_entry 未確定間の出力フレームを保留する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-encoder-defer-output-until-sample-entry-ready
- Polished:

## 目的

issue 0051 で writer 入口の sample_entry fallback 補完経路を全削除した結果、エンコーダ側の暗黙前提「最初の出力フレームが必ず keyframe で SPS / PPS が揃う」が崩れた場合のフェイルセーフが失われた。openh264 / VideoToolbox 両エンコーダで「sample_entry 未確定の間は出力フレームを保留する」設計に変更し、入力側不変条件（圧縮フレームには常に sample_entry を付与）を実装レベルで堅牢に守る。

## 優先度根拠

Low。現状の実装でも VTCompressionSession / openh264 の通常動作として「最初の出力フレームが必ず keyframe」となるため実運用は破綻していない。ただし将来の API 仕様変更や挙動変化があれば、`Mp4Writer` / `HybridMp4Writer` では muxer の `MissingSampleEntry` Err で fail-fast 停止、`DashWriter` / `HlsWriter` では handle_*_frame Err が上位の `tracing::warn!` で握り潰されて不正な ADTS / AnnexB が静かに出力される（デフォルト値 length_size=4、AAC LC/48kHz/stereo 等にフォールバック）リスクが残る。本 issue はこの暗黙前提を実装レベルで解消する。

## 現状

- `src/encoder/openh264.rs:60-96`: SPS / PPS が空の場合、`last_sample_entry` を None のまま保持しつつ `output_queue.push_back` で `sample_entry: None` のフレームを下流に流す経路を持つ
- `src/encoder/video_toolbox.rs:147-183`: 同様に `sample_entry: None` のまま push する経路を持つ
- `docs/internals/sample_entry_invariant.md` の「確立できない場合の扱い」節（issue 0051 のレビュー反映で書き換え済み）で、この経路が「API 保証ではない暗黙の運用前提」に依存している旨を明示し、実装レベルでの堅牢化は別 issue（本 issue）として整理してある

## 設計方針

### 1. sample_entry 未確定間の出力フレームを内部バッファに退避

両エンコーダ（`src/encoder/openh264.rs` / `src/encoder/video_toolbox.rs`）で以下のロジックに変更する。

1. `sample_entry` が未確定（None）の間は出力フレームを内部バッファ（例: `pending_output: VecDeque<Encoded...>`）に退避し、`output_queue` には push しない
2. SPS / PPS が揃って `sample_entry` が確定したタイミングで、退避していたフレームに `sample_entry` を載せて一括 `output_queue` に push する
3. 退避バッファのサイズ上限（例: 既定 60 フレーム = 約 1 秒分 @60fps）を設定し、超えた場合はエンコーダ Err を返す（異常状態の検知）

### 2. テスト追加

両エンコーダの `mod tests`（もしくは `tests/test_encoder_*.rs`）に以下を追加する。

- 「sample_entry 確定前の出力が `output_queue` に積まれないこと」の単体テスト
- 「sample_entry 確定後に退避フレームが一括 push されること」の単体テスト
- 既存テスト（`openh264_sets_sample_entry_on_every_output_frame` 等）は引き続き通ること

### 3. ドキュメント更新

`docs/internals/sample_entry_invariant.md` の「確立できない場合の扱い」節（現状は暗黙前提の注記）を、両エンコーダで内部バッファに退避し sample_entry 確定後に一括 push する設計に揃えて書き換える。

### CHANGES.md

本 issue で変更する範囲は内部実装の堅牢化で外部 API 変更を伴わない。`shiguredo-changelog` の「派生元ブランチとの最終的な差分のみを記載すること」に従って判断する。リリース時に観測可能な挙動変化（例: Err の発生条件変化）があるなら記載、無いなら記載なし。実装時に最終判断する。

## スコープ

含むもの:

- `src/encoder/openh264.rs` の出力経路改修と単体テスト追加
- `src/encoder/video_toolbox.rs` の出力経路改修と単体テスト追加
- `docs/internals/sample_entry_invariant.md` の「確立できない場合の扱い」節の書き換え

含まないもの:

- NVENC / svt_av1 / libvpx / fdk-aac / AudioToolbox / Opus 経路（これらはコンストラクタで sample_entry を確定する設計のため対象外）
- writer 入口の fallback 復活（issue 0051 で確立した「責任の所在を入力側に集約する」方針を維持する）

## 完了条件

- openh264 / VideoToolbox の出力フレームに `sample_entry: None` が混入しないことが単体テストで保証されること
- 既存テスト（`openh264_sets_sample_entry_on_every_output_frame` 等）が引き続き通ること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `video_toolbox` を含む）
- `docs/internals/sample_entry_invariant.md` の記述が新実装と整合していること

## 関連

- closed/0051（writer 入口 fallback 削除。本 issue の前提。本 issue 着手時点で closed されている想定）
- closed/0017 / closed/0027（エンコーダの sample_entry 全フレーム付与）
- closed/0034（writer 入口 fallback の導入）
- `docs/internals/sample_entry_invariant.md`
