# 映像 sample_entry を SharedSampleEntry で全フレーム付与に統一する

- Priority: Low
- Created: 2026-06-08
- Completed:
- Model: Claude Opus 4.8
- Branch:
- Polished:

## 目的

issue 0017 で音声側の sample_entry を「全出力フレームに載せる」方式へ変更し、共通型 `SharedSampleEntry` を導入した。一方で映像側は挙動を据え置き、keyframe にのみ sample_entry を補完する方式のまま `SharedSampleEntry` で型をラップしただけになっている。

本 issue では映像側も音声と同じく「全出力フレームに sample_entry を載せる」方式へ統一し、映像・音声で sample_entry の付与ポリシーを一本化する。これにより issue 0017 が据え置いた映像側の補完責務（0017 の決定 1 で別 issue に切り出すとした部分）を確定させ、後続の非 Option 化（issue 0028）の前提を整える。

**前提**: 本 issue は issue 0017 の完了を前提とする。`SharedSampleEntry` の導入と音声側の全フレーム付与が済んでいることが必要。

## 優先度根拠

Low。issue 0017 で映像側も `SharedSampleEntry` 型に揃っており、keyframe 補完によって muxer の契約（最初のサンプルに sample_entry 必須）は満たされているため、機能的なバグは無い。本 issue は付与ポリシーを音声と揃えて将来の非 Option 化（issue 0028）を可能にするための仕上げであり、時間があるときに対応する。

## 現状

- `VideoFrame.sample_entry`（`src/video.rs:50`）は issue 0017 完了後も生の `Option<SampleEntry>` のまま（0017 は差分最小化のため音声側 `AudioFrame.sample_entry` だけを `Option<SharedSampleEntry>` 化し、映像には手を入れていない）。共通型 `SharedSampleEntry`（`src/sample_entry.rs`）は 0017 で導入済みで、本 issue から利用できる。
- 映像エンコーダは sample_entry を最初の出力フレームにしか載せない。録画 writer はそれを取りこぼさないよう、`push_encoded_frame_with_metrics`（`src/encoder.rs:724-739`）で keyframe のときだけ `last_video_sample_entry` から補完している。
  - 「録画開始時のキーフレーム要求」＋「keyframe には sample_entry を常に補完」で、録画 writer が subscribe した直後の keyframe に必ず entry が届くため、映像では音声のような finalize 失敗レースは顕在化していない。
- keyframe 以外の出力フレームには sample_entry が載らないため、フィールド型は `Option` のままにせざるを得ない。音声と異なり `SharedSampleEntry` 化もされていないので、本 issue で全フレーム付与とあわせて映像のフィールド型も `Option<SharedSampleEntry>` に統一する。

## 設計方針

- `VideoFrame.sample_entry`（`src/video.rs:50`）を `Option<SharedSampleEntry>` に変更し、音声と型を揃える。これに伴い映像エンコーダ（`svt_av1` / `libvpx` / `nvcodec` / `openh264` / `video_toolbox`）・映像デコーダ（`openh264` / `video_toolbox` / `nvcodec`）・各 reader/writer の映像経路・`src/encoder.rs` の `last_video_sample_entry` を `SharedSampleEntry` 経由に直す。feature gate された箇所（`nvcodec` 等）は `--features` 付きでビルド確認すること。
- 映像エンコーダの sample_entry 付与を、音声 3 エンコーダと同様に「毎フレーム `Some(self.sample_entry.clone())`（Arc clone）を載せる」方式へ変更する。
- これに伴い `push_encoded_frame_with_metrics`（`src/encoder.rs:724-739`）の「keyframe のときだけ補完する」分岐（issue 0017 が据え置いた映像の補完責務）を撤去し、補完責務をエンコーダ側へ一本化する。
- writer 側の映像補完経路を整理する。補完を持つのは `hls` / `dash` / `mp4/hybrid` で、標準 `mp4`（`src/mp4/writer.rs`）は passthrough（補完なし）である点は音声側（issue 0017）と同じ。全フレーム付与後は映像専用の補完が不要になるため、各 writer の映像 `or_else` 補完を除去し、issue 0017 で `SharedSampleEntry` に用意した `changed_since` による変更検知へ寄せる（muxer 渡し経路へのフィルタ適用は音声と同じく optional で、muxer 側が dedup するため correctness 要件ではない）。
- 全映像エンコーダ（VP8/VP9/AV1/H.264/H.265、ソフト・ハード各実装）が「最初の出力フレームで sample_entry を確定する」不変条件を満たすことを確認する。

## 完了条件

- `VideoFrame.sample_entry` が `Option<SharedSampleEntry>` になり、音声と型が揃うこと。
- 映像エンコーダが全出力フレームに sample_entry を載せること。
- `push_encoded_frame_with_metrics` の keyframe 限定補完が撤去され、補完責務がエンコーダ側に一本化されること。
- 各 writer の映像専用の `or_else` 補完経路が無くなること（標準 `mp4` は元々 passthrough のため対象外）。
- 全映像エンコーダで「最初の出力フレームで entry が確定する」不変条件が満たされることを確認すること。
- 映像エンコーダが全出力フレームに sample_entry を載せる不変条件を単体テストで検証する（本リポジトリには PBT 基盤（pbt クレート・proptest）が無いため、`tests/*_tests.rs` の統合テストと `#[cfg(test)]` 単体テストで行う。issue 0017 の「テスト」節と同じ方針）。
- 録画機能（特に短時間録画・録画開始直後の映像トラック）にリグレッションが無いこと。

## 関連

- issue 0017（音声側の全フレーム付与と共通型 `SharedSampleEntry` 導入。本 issue の直接の前提）
- issue 0028（本 issue 完了後に着手する sample_entry フィールドの非 Option 化。本 issue がその前提）
