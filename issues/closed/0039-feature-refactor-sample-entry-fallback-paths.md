# writer 側 sample_entry fallback 補完経路の削除可能性を調査する

- Priority: Low
- Created: 2026-06-17
- Completed: 2026-06-22
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

## 解決方法

### 結論

writer 入口の sample_entry fallback 補完経路は **全削除可能** と判定した。
ただし `HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(ref sample_entry) = pending.sample_entry` ガード（`src/mp4/hybrid_writer.rs:403` / `:419`）は独立した「ベストエフォート設計」の動機を持つため**現状維持**を推奨する（fallback 補完経路の有無に依存しない判断）。

### 調査結果

#### 入力側不変条件の確立状況

- リーダー側: issue 0030（mp4 / RTSP / SRT AAC 音声）/ 0031（WebM）/ 0032（RTSP Annex-B 映像）/ 0033（SRT Annex-B 映像）で全て確立済み（全 closed）
- エンコーダ側: issue 0017（音声エンコーダ全フレーム付与）/ 0027（映像エンコーダ全フレーム付与）で確立済み（全 closed）
- issue 0034 当時「現実に違反が流入し得る」とされた dash / hls の WebM / Annex-B 上流も、0031-0033 完了で不変条件下に入った

`sample_entry: None` の代入が残る箇所は全て (1) 生フォーマット（`format.codec_name().is_none()`）、(2) テストフィクスチャ、(3) 構成情報（`VideoTrackConfig` 等）のいずれか。圧縮フレームが writer 入口に届く経路で `sample_entry: None` になるパスは無い。

#### Patched / Skip の発火経路

`SampleEntryResolution::Patched` / `Skip` を発火させるのはテストコードのみ:

- `src/sample_entry.rs` 単体テスト 10 件
- `pbt/tests/prop_sample_entry.rs` PBT 8 件
- `src/mp4/hybrid_writer.rs` 単体テスト 8 件（issue 0034 で新規追加）

本番経路で発火しない設計（4 writer × 2（音声 / 映像）× 2（Patched / Skip）= 16 サイト全てに `tracing::warn!` "encoded-frame invariant violated"）であり、デッドコードに近い状態。

#### 「将来の入力経路変更への保険」の残存価値

issue 0034 が「将来の入力経路変更への保険」として導入した動機は以下の点から削除側に倒すのが妥当と判断した。

- 将来の入力経路追加時は 0030-0033 と同様に **入力側で不変条件を満たす** べき（writer 側に押し付けると責任の所在が曖昧になる）
- 違反が起きた場合は PBT / 単体テスト / e2e で検知される
- writer 側で「念のため」の保険を恒久的に保持するコストは継続的にかかる（per-frame `&mut` 借用、Arc 更新、warn 文字列、16 サイトの match 重複）

#### `maybe_flush_initial_pending` のガードの位置づけ

`HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(ref sample_entry) = pending.sample_entry` ガード（`src/mp4/hybrid_writer.rs:403` / `:419`）は、fallback 補完経路の有無に直接依存しない。

- 「pending → リカバリ用 moov 先行更新」のベストエフォート経路で、失敗時に panic しない設計
- writer 入口の fallback を削除しても、pending.sample_entry は入力側不変条件で常に Some
- 「ベストエフォート＝失敗時に panic しない」という独立した設計動機を持つため、`if let Some` パターンは維持できる

後続 issue では「ガードは残し、コメント文言だけ書き換える」方針を採用する（現状コメント `src/mp4/hybrid_writer.rs:398-401` の「writer 入口の fallback で sample_entry が補完済み」を「入力側不変条件で常に Some」に直す）。

### 後続 issue

以下 1 本を後続として起票する（依存関係上、enum・関数・フィールド・呼び出し・テストを同時に消さないとビルドが通らないため分割不可）。

**`writer 入口の sample_entry fallback 補完経路を削除する`（refactor 系）**

- ブランチ名案: `feature/refactor-remove-writer-sample-entry-fallback`
- 削除対象:
  - `src/sample_entry.rs` の `resolve_audio_sample_entry` / `resolve_video_sample_entry` / `SampleEntryResolution<T>` enum と単体テスト 10 件
  - `Mp4Writer` / `HybridMp4Writer` / `DashWriter` / `HlsWriter` の `fallback_audio_sample_entry` / `fallback_video_sample_entry` フィールド 8 個と各 writer 入口の match 処理 8 サイト
  - `src/mp4/hybrid_writer.rs` の単体テスト 8 件（`hybrid_writer_falls_back_on_missing_sample_entry_*` / `hybrid_writer_skips_first_frame_when_missing_sample_entry_*` / `hybrid_writer_resolves_sample_entry_even_when_*_track_id_is_disabled` / `hybrid_writer_preserves_fallback_across_consecutive_violations_*`）
  - `pbt/tests/prop_sample_entry.rs` 全件（ファイル削除）と `pbt/Cargo.toml` の `shiguredo_mp4` dev-dependency（issue 0034 で追加された分）
- 残す対象:
  - `HybridMp4Writer::maybe_flush_initial_pending` の `&& let Some(...)` ガード（コメント文言だけ書き換え）
  - `SharedSampleEntry::ptr_eq`（fallback とは独立した `changed_since` 短絡経路観測用）
  - fallback と無関係な既存テスト（`hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry` 等）
- 加えて: 入力側で「**圧縮（エンコード済み）フレームには常に `sample_entry` を付与する**」不変条件を、`docs/internals/` 配下の新規ドキュメントとして明文化する（ファイル名は後続 issue で決定）。issue 0017 / 0027 / 0030 / 0031 / 0032 / 0033 / 0034 で段階的に確立した不変条件の所在をコード外に明示し、将来の入力経路追加時にこの規約を踏襲することを担保する目的。`AudioFrame.sample_entry` / `VideoFrame.sample_entry` の docstring からもこのドキュメントへリンクを張る。

### CHANGES.md（解決方法側の方針メモ）

本 issue ではコード変更を伴わないため記載しない。後続 issue で改めて記載要否を判断する。fallback コード自体は issue 0034 で develop に追加されたもので未リリースである可能性が高く、その場合は shiguredo-changelog の「派生元ブランチとの最終的な差分のみを記載すること」「開発ブランチ内の中間状態の修正は記載しないこと」に従い CHANGES.md 記載なしになる見込み。
