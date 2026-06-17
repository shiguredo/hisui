# writer 側 sample_entry fallback 補完経路の削除可能性を調査する

- Priority: Low
- Created: 2026-06-17
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-sample-entry-fallback-paths
- Polished:

## 目的

issue 0030 / 0031 / 0032 / 0033 で入力側全経路（mp4 リーダー、rtsp / srt の AAC 音声、WebM リーダー、rtsp の Annex-B 映像、srt の Annex-B 映像）に「圧縮フレームは常に sample_entry を持つ」不変条件が成立する。issue 0034 で writer 入口に保険として導入された不変条件違反検知 + fallback 補完経路が、入力側で違反が構造的に起き得なくなったことでデッドコードになっている可能性がある。本 issue では削除可能性を調査し、削除可能と判断した場合は後続 refactor / change issue を起票して実施する。

## 優先度根拠

Low。本 issue は調査のみで、削除可否の判定後に別 issue で実施する。fallback 補完経路は writer 入口の二重防護として動作しているため、削除しなくても実害は無い。ただし、入力側で不変条件が確立した今、保険として残すか「不要として削除する」かの設計判断を整理する価値がある。

## 現状

issue 0034 で導入された writer 入口の不変条件違反検知 + fallback 補完経路:

- `src/sample_entry.rs:107-161`: `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数。`SampleEntryResolution::{Pass, Patched, Skip}` enum で writer 側の処理方針を表現
- 各 writer の `fallback_*_sample_entry` フィールドと呼び出し:
  - `src/mp4/writer.rs`: `Mp4Writer`
  - `src/mp4/hybrid_writer.rs`: `HybridMp4Writer`。特に `maybe_flush_initial_pending` の `&& let Some(...)` ガード（issue 0034 で「将来の入力経路変更への保険として残す」と判断された）
  - `src/dash/writer.rs`: `DashWriter`
  - `src/hls/writer.rs`: `HlsWriter`

不変条件成立の経緯:

- issue 0030: mp4 リーダー / rtsp / srt の AAC 音声経路への適用と writer 側補完ロジック削除（closed）
- issue 0031: WebM リーダー経路への拡張
- issue 0032: rtsp の Annex-B 映像経路への拡張（closed）
- issue 0033: srt の Annex-B 映像経路への拡張（closed）

これにより、`VideoFrame.sample_entry` / `AudioFrame.sample_entry` の不変条件 docstring の例外節（`src/audio.rs:92` / `src/video.rs:56` の「現時点で未適用の経路: WebM リーダー。」）が 0031 完了で消えて、入力側全経路で `Some(SharedSampleEntry)` が保証される。

## 調査対象

- `src/sample_entry.rs`: `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数、`SampleEntryResolution` enum、関連単体テスト
- `src/mp4/writer.rs`: `Mp4Writer` 内の fallback フィールド・呼び出し
- `src/mp4/hybrid_writer.rs`: `HybridMp4Writer` 内の fallback フィールド・`maybe_flush_initial_pending` の `&& let Some(...)` ガード
- `src/dash/writer.rs`: `DashWriter` 内の fallback フィールド・呼び出し
- `src/hls/writer.rs`: `HlsWriter` 内の fallback フィールド・呼び出し

## 調査対象外

decoder 側の sample_entry 抽出コード:

- `src/decoder/openh264.rs` / `src/decoder/video_toolbox.rs` / `src/decoder/nvcodec.rs`: SPS / PPS / VPS 抽出
- `src/decoder/fdk_aac.rs` / `src/decoder/audio_toolbox.rs`: AudioSpecificConfig 抽出

これらは「不変条件が満たされたフレームから sample_entry を取り出して decoder 初期化に使う」処理で、不変条件適用と削除可否は無関係。むしろ「常に sample_entry が来る前提で `ok_or_else` で Err にしている既存コードがより堅牢になる」関係。

## 設計方針

### 1. 削除可否の調査

各 writer の fallback 補完経路を読み、以下を判定する:

- `fallback_*_sample_entry` フィールドへの代入が「不変条件違反フレームの補完」以外の用途で使われていないか（warn ログ・metrics・テスト等）
- `resolve_*_sample_entry` の `Patched` / `Skip` バリアントが実運用で発火する経路があるか（grep でログ出力箇所・テストフィクスチャを確認）
- `maybe_flush_initial_pending` の `&& let Some(...)` ガードの目的（issue 0034 で「将来の入力経路変更への保険」とした判断）が、入力側全経路で不変条件が確立した今も妥当か

### 2. 判定結果に応じた次アクション

- 全 writer で削除可能と判断: refactor 系後続 issue を起票して各 writer / `src/sample_entry.rs` の不要コードを一括削除。`SampleEntryResolution` enum も削除候補
- 一部 writer のみ削除可能と判断: 安全な範囲のみ削除する refactor 系後続 issue を起票
- 削除不可と判断（例: 将来の入力経路変更への保険として残すべき）: 本調査 issue の解決方法に判定根拠を記して close

## 完了条件

- 上記調査対象の各箇所の削除可否判定が本 issue の解決方法に記されていること
- 削除可能なら後続 issue が起票されていること（本 issue は調査 issue でコード変更は伴わない）
- 削除不可なら判定根拠が明示されていること

### CHANGES.md

本 issue では記載しない（調査のみ。コード変更は後続 issue で実施し、その時点で記載要否を判断する）。

## 関連

- issue 0030（入力側全経路への不変条件適用の起点）
- issue 0034（writer 入口の保険として fallback 補完を導入）
- issue 0031（WebM リーダーへの不変条件拡張。本 issue の前提）
- issue 0032 / 0033（RTSP / SRT Annex-B 映像への不変条件拡張）
