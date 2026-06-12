# sample_entry カウンタ観測 API を廃止して writer 不変条件違反検知に置き換える

- Priority: Low
- Created: 2026-06-10
- Completed: 2026-06-12
- Model: Claude Opus 4.7
- Branch: feature/change-encoded-frame-sample-entry-counters
- Polished: 2026-06-12

## 目的

`Mp4WriterStats` の `received_*_sample_entry_count` / `missing_*_sample_entry_count` カウンタ系列（`pub fn` ゲッターおよび Prometheus メトリクス `hisui_total_*_sample_entry_count` を含む）は、エンコード済みフレームが常に sample_entry を持つ不変条件をリーダー / AAC 音声入力経路と writer 補完削除に適用したことで、観測価値を失った。本 issue ではこれらを破壊的に削除し、代替として writer 入口に不変条件違反検知ロギング（`tracing::warn!`）と違反フレーム救済のフォールバック保持を導入する。

「廃止」と「置き換え」を 1 PR で同時に実施する理由は、削除のみ先行すると違反検知の死角期間（カウンタ廃止 → 違反検知導入の間）が生じ、後続予定の WebM / Annex-B 経路（別経路で不変条件未適用）からの違反流入を捕捉できなくなるため。

公開 API（`pub fn` ゲッター）と `/metrics` Prometheus メトリクスの削除を含む破壊的変更であるため `feature/change-` プレフィックスで対応し、CHANGES.md に CHANGE エントリを追加する。

## スコープ

含むもの:

- `Mp4WriterStats` の sample_entry 観測 API 系列（フィールド・カウンタ初期化・struct 初期化・`add_*` メソッド・`pub fn` ゲッター）の削除
- `/metrics` Prometheus メトリクス `hisui_total_*_sample_entry_count` 4 種と、`compose --stats-file` / `--emit-exit-metrics` 出力 JSON 中の同名 4 メンバーの削除（いずれもカウンタ初期化削除により自動消滅する）
- `HybridMp4Writer` 内の `last_*_sample_entry` 保持フィールド・初期化・`handle_*_message` 内 `changed_since` 判定・`add_received_*_sample_entry` 呼び出し・関連 received 系テスト 2 件の削除
- 4 writer（mp4 / hybrid / dash / hls）の入口への違反検知 `tracing::warn!` 追加と、違反フレーム救済のフォールバック保持 `fallback_*_sample_entry` の追加
- CHANGES.md `## develop` への [CHANGE] エントリ追加
- 本 issue で触るファイル（`src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs`）内の `issue NNNN` コメント参照を、消えるコードと共に削除する

含まないもの:

- `total_finalize_success_count` / `total_finalize_failure_count` カウンタおよび Prometheus メトリクス（finalize の成否を観測する唯一の手段で、回帰検知用途として恒久的に有用）
- `AudioFrame.sample_entry` / `VideoFrame.sample_entry` のフィールドコメント（不変条件記述）。本 issue は writer 側のみの整理であり、frame 構造体側は WebM / Annex-B 経路の不変条件適用 issue（後続）で調整される。これらコメント内に残る `issue 0031` / `issue 0032` / `issue 0033` 参照、および既存テスト（例: `hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry`）のコメント内に残る `issue 0017` / `issue 0030` 参照は shiguredo-issues 規約違反だが、本 issue のスコープ外で別 issue（既存負債清算用）に委ねる

## 優先度根拠

Low。`received_*_sample_entry_count` は不変条件下で「そのトラックでフレームを 1 つでも受信したか」を表す 0/1 値の指標に縮退し（`total_received_*_data_count` で代替可能）、`missing_*_sample_entry_count` は常に 0 固定値として `/metrics` に残り続け、運用者を混乱させる。

## 現状

エンコード済みフレーム sample_entry 不変条件の適用と writer 補完削除は完了済み（develop ブランチ b7abd458 時点）。残された対象を概念単位で示す。具体的な行番号と参照箇所一覧は実装着手時に「影響範囲確認」節の grep コマンドで再特定する。

廃止対象（観測 API 系）:

- `Mp4WriterStats` のフィールド 4 種（`total_received_audio_sample_entry_count` / `total_received_video_sample_entry_count` / `total_missing_audio_sample_entry_count` / `total_missing_video_sample_entry_count`）と、それらの `stats.counter("total_*_sample_entry_count")` 初期化・struct 初期化
- `pub(crate) fn add_received_*_sample_entry` と `pub(crate) fn add_missing_*_sample_entry` メソッド（後者は writer 補完削除で呼び出し元が消えており、`#[expect(dead_code)]` 属性で抑制された状態）
- `pub fn total_received_*_sample_entry_count()` / `total_missing_*_sample_entry_count()` ゲッター 4 種
- Prometheus メトリクス `hisui_total_received_*_sample_entry_count` / `hisui_total_missing_*_sample_entry_count`（`pipeline_handle.stats().to_prometheus_text()` 経由で `/metrics` HTTP エンドポイントに動的出力されている）

廃止対象（`HybridMp4Writer` 内の付随コード）:

- `last_audio_sample_entry: Option<SharedSampleEntry>` / `last_video_sample_entry` フィールドと初期化（`changed_since` 判定の前回値保持）
- `handle_audio_message` / `handle_video_message` 内の `changed_since` 判定と保持取り込みブロック（`add_received_*_sample_entry()` を呼ぶ唯一の経路）
- `Mp4Writer`（標準 MP4・compose / record 経路）からは `add_received_*_sample_entry` は一切呼ばれないため、こちら側に廃止対象の付随コードは無い

writer ごとの上流経路と違反検知の意義:

- `Mp4Writer` / `HybridMp4Writer`: 上流はエンコーダまたは mp4 リーダー経路で、不変条件は適用済み。違反は基本起きないが、将来配線変更（特に WebM リーダー直結等）に対する保険として違反検知と fallback を入れる
- `DashWriter` / `HlsWriter`: obsws 配線（録画 + ライブ配信）。上流は WebM リーダー・rtsp / srt の Annex-B 映像経路（不変条件未適用）を含むため、違反が現実に流入し得る

writer の Err 取扱いの差異:

- `DashWriter::run` / `HlsWriter::run` は `handle_*_frame` の Err を `tracing::warn!` で握り潰して継続する（ライブ配信 SLA: 1 フレームの違反で配信全体を停止しない）
- `Mp4Writer::run` / `HybridMp4Writer::run` の `handle_*_message` Err は `run` の外に伝播し processor を停止させる fail-fast 寄りの設計

## 設計方針

### 1. カウンタ・メソッド・関連フィールド・テストの削除

`src/mp4/writer.rs` から削除:

- `Mp4WriterStats` の sample_entry 系 4 フィールド・`stats.counter("total_*_sample_entry_count")` 初期化・struct 初期化・直前のフィールド説明コメント
- `pub(crate) fn add_received_*_sample_entry` と `pub(crate) fn add_missing_*_sample_entry`（`#[expect(dead_code)]` 属性も同時撤去）
- `pub fn total_received_*_sample_entry_count()` / `total_missing_*_sample_entry_count()` ゲッター 4 種

`src/mp4/hybrid_writer.rs` から削除:

- `last_audio_sample_entry: Option<SharedSampleEntry>` フィールドと `last_video_sample_entry` フィールド
- 構造体初期化の `last_audio_sample_entry: None` / `last_video_sample_entry: None` 行
- `handle_audio_message` 内の `changed_since` 判定・保持取り込み・`add_received_audio_sample_entry()` 呼び出しブロック全体
- `handle_video_message` 内の同等ブロック全体
- 既存テスト 2 件: `hybrid_writer_received_audio_sample_entry_counts_only_changes`、`hybrid_writer_received_video_sample_entry_counts_only_changes`

呼び出し元と関数本体は同一コミットで一括削除し、`cargo clippy --deny warnings` の dead_code を回避する。

`src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs` で削除する箇所に付随する `issue NNNN` 参照コメントはコードと一緒に消える。本 issue が削除しない周辺コメントに別 issue 番号への参照が残っている場合は、可能な範囲で「理由そのもの」のコメント（例: 「エンコード済みフレームは常に sample_entry を持つ前提のため」）に置き換える。完全駆逐は別 issue（既存負債清算）に委ねるが、本 issue を完了することで新たな `issue NNNN` 参照が増えないようにする。

### 2. 違反検知ロギングの追加

挿入の前提:

- 対象はエンコード済み（圧縮）フレームのみ。`frame.format.codec_name().is_some()` を先にガードしてから `frame.sample_entry.is_none()` を判定する（生フォーマットは writer 入口に来ない設計だが防御的に判定する）
- 出力は `tracing::warn!`。fail-fast Err は採用しない（理由は「現状」節「writer の Err 取扱いの差異」参照）
- レートリミットは入れない（違反は基本起きないはずで、起きた場合は早期検出のために全件出す）
- key=value 形式で `format` と `timestamp_us` を含める。`track_id` は writer 種別ごとに取得経路が異なる（dash / hls の `handle_*_frame` シグネチャからは取れない）ため採用しない。writer 種別は既存 `processor_type` 命名（`mp4_writer` / `hybrid_mp4_writer` / `dash_writer` / `hls_writer`）と一致する prefix をログ本文文字列で示す

挿入位置（4 ファイル × 音声 / 映像 = 8 サイト。すべての早期 return より前に挿入する。違反検知後の `Ok(())` パス（skip）も `total_input_*_frame_count` には計上され、受信観測の連続性を保つ）:

- `src/mp4/writer.rs::Mp4Writer::handle_audio_message` / `handle_video_message`: `crate::Message::Media(...)` アーム内の `add_received_*_data` 直後（`input_*_track_id.is_some()` ガードより前）
- `src/mp4/hybrid_writer.rs::HybridMp4Writer::handle_audio_message` / `handle_video_message`: 同一位置（削除した `changed_since` ブロックの跡地）
- `src/dash/writer.rs::DashWriter::handle_audio_frame` / `handle_video_frame`: 関数冒頭の `total_input_*_frame_count.inc()` 直後（コーデック確定ブロック、セグメント未開始時の早期 return、非キーフレーム時の早期 return より前）
- `src/hls/writer.rs::HlsWriter::handle_audio_frame` / `handle_video_frame`: 同一位置

`input_*_track_id` ガードより前に置くのは、track 無効化中の受信フレームも違反観測の対象に含めることで観測の連続性を保つため。HybridMp4Writer については既存の `changed_since` ブロックがガード前にあったのと挿入位置として整合する。

ログ仕様の例（hybrid_mp4_writer の audio frame の場合）:

```rust
tracing::warn!(
    format = ?sample.format,
    timestamp_us = sample.timestamp.as_micros() as u64,
    "hybrid_mp4_writer audio frame without sample_entry; encoded-frame invariant violated"
);
```

`timestamp_us` は `as_micros()` の `u128` 結果を `u64` にキャストする（実用上の録画 / 配信時間範囲ではオーバーフローしない）。本 issue で新規導入する違反検知ログのみ key=value 形式を採用する（既存 writer の format 文字列方式 warn ログとは非対称になるが、フィールド単位での検索性を優先）。

### 3. 違反フレームのフォールバック保持（補完値方式）

違反フレーム（`frame.sample_entry.is_none()` で且つ圧縮フォーマット）の救済として、直前の sample_entry を一時保持する `fallback_*_sample_entry` フィールドを各 writer に追加する。

追加先と命名:

- `Mp4Writer` / `HybridMp4Writer` / `DashWriter` / `HlsWriter` の構造体に `fallback_audio_sample_entry: Option<SharedSampleEntry>` / `fallback_video_sample_entry: Option<SharedSampleEntry>` を追加する。初期値 `None`
- `HlsWriter` は `MpegTsState` と `Fmp4State` 両方で sample_entry を使うが、両 state 共通で同じ fallback 値を使えるため、フィールドは `HlsWriter` 直下に 1 ペアだけ追加する（state 内には追加しない）
- `HybridMp4Writer` 側の `fallback_*_sample_entry` 追加と「廃止対象」の `last_*_sample_entry` 削除は同一コミットで行う。フィールド名と用途コメントを「フォールバック専用」と明示し、削除した `last_*` の名前は再利用しない

更新と参照のロジック（各 writer 入口、エンコード済みフレームのみ対象。違反検知ログ → fallback 適用 / skip の順）:

1. 圧縮フォーマット判定: `frame.format.codec_name().is_none()` の場合は通常パスへ落とす
2. 通常パス（`frame.sample_entry.is_some()`）: `self.fallback_*_sample_entry = frame.sample_entry.clone()` で保持を更新したうえで、`frame.sample_entry` を以後の append / muxer ロジックに渡す（`frame.sample_entry` の型は `Option<SharedSampleEntry>` のため `Some` で包み直さない）
3. 違反パス（`frame.sample_entry.is_none()`）:
   - 必ず先に `tracing::warn!` を出す
   - `fallback_*_sample_entry` が `Some` なら補完して以後のロジックに渡す
   - `None`（トラック先頭フレームから違反が発生し fallback が未確立）なら当該フレームを skip して `Ok(())` を返す。skip 時は `last_*_timestamp` 等の状態更新も行わない

違反パスでのフレーム差し替え実装パターン（`MediaFrame::Audio(Arc<AudioFrame>)` / `MediaFrame::Video(Arc<VideoFrame>)` は Arc 包装）:

- Mp4Writer / HybridMp4Writer（`Arc<AudioFrame>` を受け取る側）:
  ```rust
  let fb = self
      .fallback_audio_sample_entry
      .clone()
      .expect("fallback must be Some here by the is_some() check above");
  let patched = AudioFrame { sample_entry: Some(fb), ..(*sample).clone() };
  self.core.handle_input_sample(
      InputTrackKind::Audio,
      Some(crate::MediaFrame::Audio(Arc::new(patched))),
  )?;
  ```
  この `(*sample).clone()` は `AudioFrame` の deep copy（`data: Vec<u8>` を含む）。さらに後続の `prepare_audio_for_queue` 内でも `sample.as_ref().clone()` で再度 deep copy が走るため、違反パスは合計 2 回の deep copy になる。違反は基本起きない前提なのでコストは許容する
- DashWriter / HlsWriter（`&AudioFrame` を受け取る側）:
  ```rust
  let fb = self
      .fallback_audio_sample_entry
      .clone()
      .expect("fallback must be Some here by the is_some() check above");
  let patched = AudioFrame { sample_entry: Some(fb), ..frame.clone() };
  let frame = &patched;
  ```
  以後の参照経路を `&patched` 側に shadow して使う

issue 0030 との整合: 0030 は writer 側の `.or_else()` 常時パスフォールバックを「事実上デッド」として削除した。本 issue の `fallback_*_sample_entry` は (i) 違反検知の warn ログを先に出す、(ii) 違反時のみ消費される、という構造的差を持ち、0030 の判断と矛盾しない。

`maybe_flush_initial_pending`（0030 closed L131）の `&& let Some(ref sample_entry)` ベストエフォートガードは、本 issue の writer 入口での違反検知 + fallback 適用により事実上常に通過するようになるが、設計意図（将来の入力経路変更への保険）は保つため改変しない。

### 4. CHANGES.md エントリ追加

`## develop` の CHANGE 群末尾（[ADD] グループの直前）に以下の [CHANGE] エントリを追加する。shiguredo-changelog 規約（`- [種別] 〜を〜する` 形式、種別順、担当者行）に準拠する。

```
- [CHANGE] エンコード済みフレームの sample_entry 観測 API（Prometheus メトリクスおよび Rust 公開ゲッター）を削除する
  - Prometheus メトリクス 4 種（`hisui_total_received_audio_sample_entry_count` / `hisui_total_received_video_sample_entry_count` / `hisui_total_missing_audio_sample_entry_count` / `hisui_total_missing_video_sample_entry_count`）を削除する
  - `Mp4WriterStats` の公開ゲッターメソッド 4 種（`total_received_audio_sample_entry_count()` / `total_received_video_sample_entry_count()` / `total_missing_audio_sample_entry_count()` / `total_missing_video_sample_entry_count()`）を削除する
  - `compose --stats-file` 出力 JSON および `--emit-exit-metrics` 出力からも同名 4 メンバーが削除される
  - 上流のエンコード済みフレームが常に sample_entry を持つ不変条件を適用したため、received は「そのトラックでフレームを 1 つでも受信したか」を表す 0/1 値の指標に縮退し `total_received_*_data_count` で代替可能、missing は常に 0 固定値となり観測価値を失ったため
  - 代替として writer 入口で sample_entry 欠落フレームを警告ログとして出力するようにする
  - @sile
```

CHANGES.md エントリ内に issue 番号への参照を書かない（shiguredo-issues 規約）。担当者ハンドル `@sile` は実装担当者に応じて差し替える。

## 完了条件

- 設計方針 1〜4 の削除・追加が実装されていること
- `feature/change-` プレフィックスのブランチで実装され、CHANGES.md `## develop` に [CHANGE] エントリ（issue 番号への参照を含まない）が追加されていること
- 後述「テスト」節の新規テスト（hybrid_writer 4 件 + endpoint_http_metrics 1 件）が追加されていること
- 「影響範囲確認」節の grep コマンドの結果が、削除対象シンボル名・`src/mp4/hybrid_writer.rs` の `last_*_sample_entry` ともに空であること
- 本 issue で触るファイル（`src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs`）から、削除されたコードに付随する `issue NNNN` 形式の参照コメントが消えていること
- compose 経路の `--stats-file` 出力 JSON および `--emit-exit-metrics` 出力から削除対象 4 メンバーが消えていることを実機実行で確認する
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）
- 既存 e2e テスト（`e2e-tests/obsws/test_output.py` 等の HLS / DASH / MP4 / SRT 関連）が通ること

### テスト

新規テストは「`handle_*_message` で違反フレームを投入したときに fallback と入力キューが期待通りになっているか」を直接観測する。投入経路は既存 `hybrid_writer_received_*_sample_entry_counts_only_changes` テストと同じ `handle_audio_message` / `handle_video_message` 直接呼び出しパターンを採用し、検証は `writer.core.input_*_queue` の中身（補完後の sample_entry）と `writer.fallback_*_sample_entry` の状態に対する assert で行う。finalize 後の読み戻し統合検証は既存 `hybrid_writer_finalizes_readable_streams_with_per_frame_sample_entry` でカバーされているため重複させない。`tracing::warn!` のログ assertion は hisui に確立した前例が無いため採用しない（fallback の状態と入力キューの中身で違反検知の動作を間接的に確認できる）。

`src/mp4/hybrid_writer.rs` のテストモジュールに以下 4 件を追加:

- `hybrid_writer_falls_back_on_missing_sample_entry_audio`: 音声フレーム 2 つ（1 つ目 `sample_entry: Some(entry)` / 2 つ目 `sample_entry: None`）を順に `handle_audio_message` で投入する。投入後、`writer.core.input_audio_queue` に 2 つの AudioFrame が積まれており、両方の `sample_entry` が同一実体を指していること（`SharedSampleEntry::changed_since` で確認）を assert。さらに `writer.fallback_audio_sample_entry` が `Some(entry)` になっていることを assert
- `hybrid_writer_falls_back_on_missing_sample_entry_video`: 映像で同様
- `hybrid_writer_skips_first_frame_when_missing_sample_entry_audio`: 1 つ目を `sample_entry: None` で投入。`writer.core.input_audio_queue` が空（先頭フレームが skip された）であり `writer.fallback_audio_sample_entry` も `None` のままであることを assert。続けて 2 つ目を `Some(entry)` で投入し、`input_audio_queue` に 1 つだけ積まれ `fallback_audio_sample_entry` が `Some(entry)` になっていることを assert
- `hybrid_writer_skips_first_frame_when_missing_sample_entry_video`: 映像で同様

`src/endpoint_http_metrics.rs` のテストモジュールに以下 1 件を追加:

- `metrics_endpoint_does_not_include_removed_sample_entry_counters`: `MediaPipeline` を生成し `Mp4WriterStats::new` を**明示的に**呼んで初期化したうえで `/metrics` レスポンス body に対し、削除対象 4 メトリクス名（`hisui_total_received_*_sample_entry_count` / `hisui_total_missing_*_sample_entry_count`）が含まれないことを `!body.contains(...)` で assert する。`Mp4WriterStats::new` を必ず呼ぶ意図は「将来 `Mp4WriterStats::new` 内で対象カウンタを再追加した場合の回帰検知」

`Mp4Writer`（標準 MP4）の単体テスト追加は省略する。`HybridMp4Writer::handle_*_message` と `Mp4Writer::handle_*_message` は構造が同型（共に `WriterCore::handle_input_sample` へ委譲する直前で違反検知と fallback を行う）であり、hybrid 側のテストで挙動が代表される。

`DashWriter` / `HlsWriter` の単体テスト追加も省略する。`DashWriter` には `#[cfg(test)] mod tests` 枠は存在するが（combined MPD 生成ヘルパ用）、`DashWriter` 本体インスタンスを起動して `handle_*_frame` を直接呼ぶ前例は無い。`HlsWriter` には `#[cfg(test)] mod tests` 枠自体が無い。fallback ロジックは hybrid_writer と同型のため `obsws/test_output.py` 系の e2e で間接カバーする。将来 issue 0031 / 0032 / 0033 で実違反経路が増えたときに dash / hls の writer インスタンステスト枠組み追加と合わせて違反シナリオの直接テストを検討する。

### 影響範囲確認

実装着手前と完了時に以下を grep して影響範囲を確認する（完了時にすべて結果が空となる）:

- `rg 'total_received_(audio|video)_sample_entry|total_missing_(audio|video)_sample_entry|add_received_(audio|video)_sample_entry|add_missing_(audio|video)_sample_entry|hisui_total_received_.*_sample_entry|hisui_total_missing_.*_sample_entry' src/ tests/ pbt/ e2e-tests/ docs/`
- `rg 'last_(audio|video)_sample_entry' src/mp4/hybrid_writer.rs`（リーダー側 `src/mp4/sample_reader.rs` / `src/mp4/reader.rs` 等の同名フィールドは 0030 で導入された保持ロジックで本 issue の削除対象外。検索範囲を `hybrid_writer.rs` に限定して誤検出を防ぐ）

## 関連

- issue 0030（直接の前提。リーダー / AAC 音声入力経路への不変条件適用と writer 補完削除。closed）
- issue 0027（映像エンコーダの全フレーム付与。closed）
- issue 0017（音声エンコーダの全フレーム付与と共通型 `SharedSampleEntry` 導入。closed）
- issue 0011（received / missing カウンタ系列の起源。closed）
- issue 0031 / 0032 / 0033（不変条件未適用経路への適用拡張。本 issue の後続）

## 解決方法

### カウンタ・メソッド・関連フィールド・テストの削除

- `src/mp4/writer.rs::Mp4WriterStats` から sample_entry 系 4 フィールド・`stats.counter("total_*_sample_entry_count")` 初期化・struct 初期化・`pub(crate) fn add_received_*_sample_entry` / `add_missing_*_sample_entry`・`pub fn total_*_sample_entry_count()` ゲッター 4 種を削除した。
- `src/mp4/hybrid_writer.rs::HybridMp4Writer` から `last_audio_sample_entry` / `last_video_sample_entry` フィールド・初期化・`handle_*_message` 内の `changed_since` 判定と保持取り込み・`add_received_*_sample_entry()` 呼び出しを削除した。
- 既存テスト `hybrid_writer_received_audio_sample_entry_counts_only_changes` / `hybrid_writer_received_video_sample_entry_counts_only_changes` を削除した。
- 上記カウンタ初期化（`stats.counter("...")`）の削除に伴い、`/metrics` Prometheus メトリクス 4 種（`hisui_total_received_*_sample_entry_count` / `hisui_total_missing_*_sample_entry_count`）と `compose --stats-file` / `--emit-exit-metrics` 出力 JSON の同名 4 メンバーは自動的に消滅する。

### 違反検知ロギングと fallback 補完値の追加

- `src/sample_entry.rs` に共通ヘルパ `SampleEntryResolution<T>` enum と `resolve_audio_sample_entry` / `resolve_video_sample_entry` 関数を追加した。圧縮フレーム判定（`codec_name().is_some()`）→ 通常パスで fallback 更新（Arc 共有）→ 違反パスで補完済みフレーム生成 or skip の 3 分岐ロジックを集約。
- `SharedSampleEntry::ptr_eq` を追加し、`changed_since` の Arc::ptr_eq 短絡経路が壊れていないかをテストで観測できるようにした。
- 4 writer（`Mp4Writer` / `HybridMp4Writer` / `DashWriter` / `HlsWriter`）に `fallback_audio_sample_entry` / `fallback_video_sample_entry` フィールドを追加し、入口で `resolve_*_sample_entry` を呼ぶように改修した。違反時は `tracing::warn!`（key=value 形式、英語、writer 種別を `processor_type` 命名と一致する prefix で識別）を出してから補完値で差し替えるか skip する。違反検知は `input_*_track_id` ガード / `total_input_*_frame_count` 計上の後、早期 return の前に配置することで観測の連続性を保つ。
- mp4 / hybrid_writer は `Arc<AudioFrame>` を `Arc::new(patched)` で詰め直し、dash / hls は `&AudioFrame` を `let patched_holder; ... &patched_holder` の shadow で借用ライフタイムを延ばす。
- `HlsWriter` の fallback は `MpegTsState` / `Fmp4State` ではなく writer 直下に 1 ペアだけ持つ（両 state 共通で同じ値を使えるため）。

### テスト

- `src/sample_entry.rs` の `mod tests` に単体テスト 10 件（音声 / 映像 × 4 分岐 + `ptr_eq` の真偽 2 件）を追加。Arc 同一性を全 Pass / Patched パスで assert することで、fallback の `Arc::ptr_eq` 短絡経路が壊れた場合（例: `entry.get().clone()` で再 wrap する実装に書き換わる）を検知可能にした。
- `pbt/tests/prop_sample_entry.rs` を新規作成し、`(codec_name 有無 × sample_entry 有無 × fallback 有無)` の 8 状態を `proptest` で性質ベースに検証する 8 件の PBT を追加。`pbt/Cargo.toml` に `shiguredo_mp4` dev-dependency を追加。
- `src/mp4/hybrid_writer.rs` の `mod tests` に 8 件のテストを追加（`falls_back_*` 音声 / 映像、`skips_first_frame_*` 音声 / 映像、`resolves_sample_entry_even_when_*_track_id_is_disabled` 音声 / 映像、`preserves_fallback_across_consecutive_violations` 音声 / 映像）。track 無効化中の違反でも fallback 更新は走ること、連続違反でも fallback が直前の正常値を保持し続けることを検証する。
- `src/endpoint_http_metrics.rs` の `mod tests` に `metrics_endpoint_does_not_include_removed_sample_entry_counters` を追加。`Mp4WriterStats::new` を明示的に呼んでも `/metrics` レスポンスに廃止 4 メトリクス名が含まれないことを確認する（将来再追加した場合の回帰検知）。

### CHANGES.md

`## develop` への [CHANGE] エントリ追加は行わなかった。本 issue で廃止する `Mp4WriterStats` の `pub fn` ゲッターおよび Prometheus メトリクスは最後のリリース以降に develop で追加されたもので、まだ正式リリースされていない。`shiguredo-changelog` の「派生元ブランチとの最終的な差分のみを記載すること」「開発ブランチ内の中間状態の修正は記載しないこと」に従い、未リリース機能の追加 → 削除は最終 diff として現れないため記載対象外と判断した。

### スコープ外として後続に委ねた項目

- **dash / hls writer の `resolve_*` 適用箇所への直接単体テスト**: writer インスタンステスト枠組み（`DashWriter` / `HlsWriter` を起動して `handle_*_frame` を直接呼ぶ仕組み）の構築が必要で本 issue の規模を超える。issue 0031 / 0032 / 0033 の実装で実違反経路が増えるタイミングで、各 PR の判断に応じて writer 側テストを併設する。
- **`SampleEntryResolution<T>` 抽象化と 8 サイトの match + tracing::warn! 重複の集約**: マクロ or trait + generics で `resolve_*` 2 関数と writer 側 8 サイトをまとめて圧縮できる（推定 150 行削減）。本 issue では実装を済ませて挙動を安定させ、リファクタは別 issue で扱う方針。
- **違反検知 warn のレートリミット**: 設計方針 2 で「違反は基本起きない前提のため全件出す」と決定済み。後続経路で実違反が観測されるようになった時点で、運用観測の別 issue として検討する。

### レビュー指摘の反映

`/review-diff-code` で挙がった指摘を順次対応した。

- コメント拡充: `let patched_holder;` の delayed-init パターンの意図、`add_received_*_data` / `total_input_*_frame_count` が違反検知前に計上済みである旨、`maybe_flush_initial_pending` が writer 入口の fallback で補完済みの想定になる旨、`Mp4WriterStats` の「ファイナライズ系カウンタは hybrid writer のみ計上」コメントの範囲明示、生フォーマット Pass で fallback を更新しない設計、Arc 共有による `changed_since` 短絡、二重 deep copy の許容を `resolve_*` の docstring に明記。
- 命名整理: `try_resolve_*_sample_entry` → `resolve_*_sample_entry`（`try_` プレフィックスは Result/Option 戻り値の Rust 慣用と齟齬があるため）。
- 軽微改善: Patched パスのログ key を `?patched.format` → `?sample.format` / `?frame.format` に統一して Skip パスと揃える、`fallback.clone()` → `fallback.as_ref()` で Skip パスの Arc clone を省略、`std::sync::Arc::new(patched)` フルパス → `use std::sync::Arc;` 追加で短縮、tracing field 名 `format` → `frame_format` に変更して `std::format!` 連想による検索性劣化を回避。
