# SRT inbound endpoint で Annex-B 映像から SPS/PPS を抽出して sample_entry を構築する

- Priority: Low
- Created: 2026-06-10
- Completed: 2026-06-15
- Model: Claude Opus 4.7
- Branch: feature/add-srt-annexb-video-sample-entry
- Polished: 2026-06-15

## 目的

SRT MPEG-TS 入力の H.264 Annex-B 映像経路に対して、IDR フレームに inline で含まれる SPS / PPS を抽出して `SharedSampleEntry` を構築し、`SrtTsDemuxer` が出力する全 H.264 映像フレームに付与する。

「エンコード済み圧縮映像フレームは下流に流れる際に常に sample_entry を持つ」という既存の不変条件を SRT Annex-B 映像経路にも拡張し、`src/video.rs` の `VideoFrame.sample_entry` docstring に残る経路例外を 1 行削減する。

## 優先度根拠

Low。本 issue は予防的整備（broken window 解消）。現状は二重防御により `sample_entry: None` が muxer 不整合を起こす経路は無い:

- 入力側: `src/srt/inbound_endpoint.rs:166-178` で `output_video_track_id.is_some()` の時に `VideoDecoder` が強制生成され、subscriber 出力は decoder を経由して I420 raw へ変換される
- writer 側: 4 writer 入口の `resolve_video_sample_entry`（0034 で導入）が `sample_entry: None` を warn + fallback / skip で吸収する

それでも対応する理由は、不変条件の境界記述（`src/video.rs:51-57` の `VideoFrame.sample_entry` docstring）に SRT Annex-B 経路の例外を残し続けることが broken window になるため。subscriber 側で sample_entry を確定させれば writer 入口の違反検知 warn が SRT Annex-B 経路から発火しないことが構造的に保証される（将来 obsws 配線が subscriber → writer 直結に変わった場合の予防にもなる）。

## カテゴリ判定

ブランチ命名は `feature/add-srt-annexb-video-sample-entry`（`add` カテゴリ）。主目的は SPS / PPS パースと sample_entry 構築機能の新規追加。`received_video_keyframe` フィールドの削除は新ゲートへの置換に伴う不可分の整理として同 issue に含める。並走する 0031 / 0032 と方針を揃える。

## 現状

行番号は HEAD（develop = edc8dbd2）時点。実装着手時は grep で再特定する（「影響範囲確認」節）。

`src/srt/inbound_endpoint.rs:940` で映像フレームは `sample_entry: None` 固定で生成される（コメント「Annex-B 入力では sample_entry は付与しない」）。

`build_video_sample`（`:895-942`）の現挙動:

- `H264AnnexBNalUnits::new(&pending.data)` ループ（`:917-923`）は IDR 判定にだけ使われる（IDR 検出時 `break` し、SPS / PPS は走査しない）
- 既存の `received_video_keyframe` ゲート（`:925-930`）は「初回 IDR を観測したか」を表し、IDR を一度見たら以後 keyframe / 非 keyframe を区別なく流す

`SrtTsDemuxer` 構造体（`:720-734`）には映像用の sample_entry 保持フィールドは存在しない（音声用 `last_aac_sample_entry: Option<SharedSampleEntry>` は `:732` に存在し、0030 で追加済み）。コンストラクタ（`:737-760`）も同様。

参照ヘルパ:

- `src/video/h264.rs:87-129` の `h264_sample_entry_from_annexb(width, height, data)` は `data` を `H264AnnexBNalUnits` で走査して SPS / PPS を抽出し、片方でも空なら `Err("missing H.264 SPS")` / `Err("missing H.264 PPS")` を返す。両方揃っていれば `SampleEntry::Avc1` を返す
- 既存の MPEG-TS 1 PES 完成判定: `complete_pes` を呼ぶ時点で `is_pes_ready` が真となり `pending.data` は 1 PES 分の完全な Annex-B バイト列になっている前提。PES 跨ぎは本 issue ではスコープ外

## 設計方針

### 1. `SrtTsDemuxer` への sample_entry 保持フィールド追加

`SrtTsDemuxer` 構造体（`:720-734`）に映像用 `last_video_sample_entry: Option<SharedSampleEntry>` フィールドを追加する。コンストラクタ（`:737-760`）で `None` に初期化する。音声側 `last_aac_sample_entry` と同方針。

フィールド docstring には issue 番号を書かない（shiguredo-issues 規約）。新規追加するコメント全般について同方針を徹底する（既存 `last_aac_sample_entry` の docstring に残る issue 番号は別途清算予定。本 issue では触らない）。

### 2. ゲート単一化（既存 `received_video_keyframe` の削除）

`received_video_keyframe`（`:733`、`:758`、`:925`、`:929` で参照）を削除し、`last_video_sample_entry.is_some()` を唯一のゲートにする。理由は AAC 側の `last_aac_config_key` / `last_aac_sample_entry` 同期更新パターンと整合し、冗長フィールドが残らないため。

ゲート位置: 現在の `:925-930` を、設計方針 3 の sample_entry 構築試行の **後** に置き、`if self.last_video_sample_entry.is_none() { return Ok(None); }` とする。これにより SPS / PPS 確定までの全フレーム（IDR / 非 IDR を問わず）が破棄される。

破棄期間延長の影響: 旧設計（`received_video_keyframe`）は最初の IDR 到達まで破棄、新設計は最初の SPS / PPS 含有 IDR 到達まで破棄。SRT MPEG-TS 入力では IDR に inline で SPS / PPS が付随するのが一般的（H.264 Annex-B エンコーダの標準的な挙動）なため、現実的な破棄期間は旧設計と同じか 1 IDR 分だけ延びる程度。上限・タイムアウトは設けない（運用上の問題が起きないと判断）。

### 3. IDR 内 SPS / PPS の抽出と sample_entry 構築

`build_video_sample`（`:895-942`）の NAL ループを以下に置き換える:

1. `H264AnnexBNalUnits::new(&pending.data)` で NAL を走査して `has_idr: bool` を立てる。IDR 検出時に `break` で打ち切る
2. `has_idr` のとき `h264_sample_entry_from_annexb(0, 0, &pending.data)?` を呼ぶ。`Ok(entry)` なら `self.last_video_sample_entry = Some(SharedSampleEntry::new(entry))` で更新する。`Err` は `?` で上位に伝播する
3. `has_idr` が false の PES（P フレームのみ）は sample_entry 構築試行をスキップし、`last_video_sample_entry` は変更しない

width / height は 0 で構築（既存 RTMP / openh264 経路も同じ）。SPS 内 Exp-Golomb 解像度抽出は本 issue ではスコープ外。

SPS / PPS 不在 IDR や破損 NAL（`H264AnnexBNalUnits` パース失敗 / `h264_sample_entry_from_annexb` の `missing H.264 SPS|PPS` Err）は、いずれもエンコーダ側の異常または伝送破損として扱い、`?` で接続を打ち切る fail-fast 方針とする。正常な H.264 ストリームは IDR に SPS / PPS を inline するのが業界標準（OBS / FFmpeg / Sora 等）で、SRT inbound endpoint は publisher 側からの一方向受信のため mid-stream joining も発生しない。同関数の上の NAL 走査 (`for nalu in ... { let nalu = nalu?; ... }`) が既に破損 NAL を `?` で伝播している既存設計と一貫させる。確定前後で挙動を分岐させたり、ログのレートリミットを設けたりする必要はない。

### 4. 全フレーム付与

`TsSample::Video(crate::VideoFrame { ... })` 構築箇所（`:934-941`）で `sample_entry: None` を以下に置き換える:

```rust
sample_entry: self.last_video_sample_entry.clone(),
```

設計方針 2 のゲートにより `last_video_sample_entry` が `None` の間はそもそも下流に流れないため、ここに到達した時点で `Some` が確定している。`build_audio_samples` 内の `let sample_entry = self.last_aac_sample_entry.clone();`（実コード `:1008` 付近）と同形に揃える。`.expect(...)` 等の `Option` 解体は使わない。

### 5. mid-stream の挙動

本節は同一接続内の mid-stream を指す。再接続時の挙動は設計方針 6 を参照。

新 IDR が SPS / PPS を含む場合、設計方針 3 のフローで `last_video_sample_entry` が新値（無条件 `SharedSampleEntry::new(...)` で新 Arc）に上書きされる。新 IDR フレーム自身に載せる sample_entry は更新後の新値。openh264 エンコーダ（`src/encoder/openh264.rs:55-67`）の挙動と同順序。

mid-stream 中に SPS / PPS 不在の IDR が来た場合は設計方針 3 の fail-fast によって `?` で接続を打ち切る。

同一 SPS / PPS の IDR が連続して新 Arc が作られても、muxer 側の `shiguredo_mp4::mux::Mp4FileMuxer` は `SampleEntry::PartialEq` で実体比較するため重複登録は起きない。AAC 側のように config 等価判定で skip する最適化は本 issue ではスコープ外。

### 6. 周辺挙動の取り扱い

- **Demuxer flush（`flush_pending`）**: `complete_pes` を呼ぶだけで `last_video_sample_entry` を直接触らない。flush 経由でも設計方針 3 のフローが回る
- **SRT 切断・再接続**: `reset_connection_state` 内で `*connection_ctx.demuxer = SrtTsDemuxer::new()?` により demuxer 全体が再生成され、`last_video_sample_entry` を含む全フィールドが初期化される。再接続後は新規接続と同じ通常フロー
- **track 無効化（`output_video_track_id` が `None`）**: `SrtTsDemuxer` は publish 経路を知らない設計のため、`output_video_track_id` の有無に関わらず `last_video_sample_entry` を更新する。SPS / PPS パースを track 無効化中も走らせるコストは µs オーダーで許容

### 7. 不変条件コメントの例外記述更新

`src/video.rs:51-57` の `VideoFrame.sample_entry` docstring から `、srt の Annex-B 映像（issue 0033）` 相当の記述を削除する。0032 の並行進行で `rtsp /` 部分が先に削られている可能性があるため、本 issue マージ時点の HEAD を Read で確認し、SRT 部分のみを diff として削る。`issue 0031` / `issue 0032` の文言や規約準拠化（issue 番号削除）は本 issue ではスコープ外（後続 issue または既存負債清算で対応）。

## 完了条件

- `SrtTsDemuxer` の H.264 映像出力フレームが全て `Some(SharedSampleEntry)` を持つこと
- SPS / PPS 含有 IDR を受信した時点で `last_video_sample_entry` が確定し、以後の全 P フレームに同じ entry が clone されて付与されること
- SPS / PPS 不在 IDR や `h264_sample_entry_from_annexb` の Err は `?` で上位に伝播し、SRT 接続を打ち切ること（確定前後を区別しない fail-fast 方針）
- mid-stream で SPS / PPS 含有 IDR が来た場合は `last_video_sample_entry` が新値に上書きされ、当該 IDR 自身に新値が載って下流に流れること
- 既存 `received_video_keyframe` フィールドが削除され、`last_video_sample_entry.is_some()` が唯一のゲートになっていること
- `src/video.rs:51-57` の `VideoFrame.sample_entry` docstring を本 issue マージ時点の HEAD で Read し、`、srt の Annex-B 映像（issue 0033）` 相当の記述のみを diff として削除すること（0032 の並行進行で `rtsp /` 部分が先に削られている場合があるため、HEAD の現状を確認した上で SRT 部分のみを対象にする）
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）
- 既存 SRT 関連 e2e テスト（`e2e-tests/obsws/test_output.py` の SRT 録画系）が通ること

### CHANGES.md

記載しない（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 / 0030 と同方針。

### テスト

新規単体テストを `src/srt/inbound_endpoint.rs` の `#[cfg(test)] mod tests` に追加する。既存 AAC 音声テスト（`srt_aac_emits_sample_entry_on_every_au_with_constant_config` / `srt_aac_updates_sample_entry_on_config_change`、`:1189-1257`）と同パターンで `build_video_sample` を直接呼ぶ。

PBT は本 issue では追加しない。`SrtTsDemuxer::build_video_sample` の状態空間（`last_video_sample_entry` の有無 × IDR 有無 × SPS / PPS の組み合わせ × NAL 並び順）は単体テスト 6 ケースで網羅可能で、`h264_sample_entry_from_annexb` 内部のパース挙動は同関数の既存テスト範囲のため。

#### テストヘルパーとフィクスチャ

- `make_h264_pending_pes(data: Vec<u8>, pts_ticks: u64) -> PendingPesPacket`: 既存の `make_aac_pending_pes`（`:1148-1165`）と同形。`dts: None` 固定で `build_video_sample` 側の `dts.unwrap_or(pts)` フォールバックを使う（AAC ヘルパとの対称性優先、PTS ≠ DTS シナリオは本 issue のテスト範囲外）。`stream_id` は `mpeg2ts::es::StreamId::new_video(StreamId::VIDEO_MIN).expect("VIDEO_MIN is valid")` を使う（AAC ヘルパは汎用 `StreamId::new` を採るが、video 側は `is_video()` 型検査で誤値混入を弾く `new_video` を選ぶ。`hls/writer.rs:599-600` 既存採用パターンに合わせる）
- SPS / PPS / IDR / P フレームの Annex-B バイト列定数を `mod tests` 内に直接埋め込む。`H264AnnexBNalUnits` は start code prefix + NAL header の `forbidden_zero_bit` のみ検査するため、payload バイト列に `0x00, 0x00, 0x01` または `0x00, 0x00, 0x00, 0x01` が含まれないことだけ保証すれば十分:
  - `SPS_A`: `[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1e, 0xab]`（NAL header `0x67` で `nal_unit_type=7` + 任意 payload 4 バイト）
  - `SPS_B`: `[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1e, 0xac]`（SPS_A の末尾 `0xab` を `0xac` に差し替え）
  - `PPS`: `[0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x06, 0xe2]`（NAL header `0x68` で `nal_unit_type=8`）
  - `IDR`: `[0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21]`（NAL header `0x65` で `nal_unit_type=5`）
  - `P_FRAME`: `[0x00, 0x00, 0x00, 0x01, 0x41, 0x9a, 0x21, 0x6c]`（NAL header `0x41` で `nal_unit_type=1`）

  各 payload バイト列に `0x00, 0x00, 0x01` シーケンスが含まれていないことは目視確認で済む（NAL ループの誤分割防止）。テストの再生成時もこの不変条件を維持する。

#### テストケース

(a) `srt_h264_emits_sample_entry_on_every_frame_after_sps_pps_idr`: `SPS_A + PPS + IDR` を含む 1 PES と `P_FRAME` のみの 2 PES を順に投入し、3 フレーム全てに `Some(SharedSampleEntry)` が載り、2 / 3 フレーム目が初回と等価（`changed_since(Some(&first)) == false`）であることを検証

(b) `srt_h264_returns_err_on_idr_without_sps_pps`: SPS / PPS を含まない IDR のみの PES を投入し、`build_video_sample` が `Err` を返す（fail-fast で `?` 伝播される）ことを検証

(c) `srt_h264_updates_sample_entry_on_mid_stream_sps_change`: `SPS_A + PPS + IDR` で初期確定したあと、`SPS_B + PPS + IDR` を投入し、新 IDR の sample_entry が初回と異なる（`changed_since(Some(&first)) == true`）こと、当該 IDR 自身に新 entry が載っていることを検証

(d) `srt_h264_emits_sample_entry_on_idr_with_trailing_sps_pps`: `[IDR, SPS_A, PPS]` 並びの PES でも sample_entry が確定して下流に流れることを検証（`h264_sample_entry_from_annexb` が PES 全体を走査することの回帰防止）

(e) `srt_h264_returns_err_on_idr_with_only_sps`: SPS のみ含む（PPS 不在）IDR で `Err` が返ることを検証

(f) `srt_h264_returns_err_on_idr_with_only_pps`: PPS のみ含む（SPS 不在）IDR で `Err` が返ることを検証

### 影響範囲確認

実装着手前と完了時に以下を grep する（着手前は現状把握、完了時は削除・追加が反映されたことを確認）:

- `rg 'sample_entry:\s*None' src/srt/inbound_endpoint.rs`: 着手前は `:940` で 1 件 hit、完了時は 0 件
- `rg 'received_video_keyframe' src/srt/inbound_endpoint.rs`: 着手前は 4 件 hit、完了時は 0 件
- `rg 'last_video_sample_entry' src/srt/inbound_endpoint.rs`: 着手前は 0 件、完了時は構造体定義 / 初期化 / 更新サイト / 参照サイト / テスト群で 5 箇所以上 hit
- `rg 'resolve_video_sample_entry' src/`: 4 writer（`mp4/writer.rs` / `mp4/hybrid_writer.rs` / `dash/writer.rs` / `hls/writer.rs`）の入口呼び出し 4 件 + `sample_entry.rs` の定義 1 件 + 同ファイル内テスト 4 件で計 9 件 hit すること（着手前と完了時で件数が変わらないことを確認する。本 issue では writer 側を変更しないため）
- `rg 'issue 003' src/srt/inbound_endpoint.rs`: 本 issue で新規追加するコメント由来の hit が無いこと（既存 `last_aac_sample_entry` docstring 由来の hit のみが残る想定）

## 関連

- issue 0030（直接の前提。リーダー / AAC 音声入力経路への不変条件適用と writer 補完削除。closed）
- issue 0034（writer 入口の `resolve_video_sample_entry` 違反検知 + fallback 補完を導入。本 issue 完了で SRT Annex-B 経路からの違反流入が構造的に消える。closed）
- issue 0027（映像エンコーダの全フレーム付与と `VideoFrame.sample_entry` の `SharedSampleEntry` 化。間接的な前提。closed）
- issue 0017（音声側の `SharedSampleEntry` 共通型導入。間接的な前提。closed）
- issue 0031（WebM リーダーへの sample_entry 構築追加。本 issue の兄弟）
- issue 0032（RTSP の Annex-B 映像 sample_entry 構築。本 issue と並行・独立で進める。`src/video.rs` の不変条件コメント編集はマージ順序により互いに影響する）

## 解決方法

### SrtTsDemuxer への sample_entry 保持フィールド追加とゲート単一化

- `SrtTsDemuxer` に `last_video_sample_entry: Option<SharedSampleEntry>` を追加し、コンストラクタで `None` 初期化した。
- 既存の `received_video_keyframe` フィールド・初期化・参照を削除し、`last_video_sample_entry.is_some()` を唯一のゲートにした。AAC 側 `last_aac_sample_entry` の同期更新パターンと整合させた。

### build_video_sample の SPS / PPS 抽出フロー

- `H264AnnexBNalUnits` での走査は IDR 検出時に `break` で打ち切り、`has_idr: bool` のみを判定する形に整理した。SPS / PPS の有無判定や PES 全体の抽出は `h264_sample_entry_from_annexb` 側の独立した走査に委ねる。
- IDR PES 全体を `h264_sample_entry_from_annexb(0, 0, &pending.data)?` に渡し、`Ok(entry)` なら `last_video_sample_entry` を新値で上書きする。SPS / PPS 不在や破損 NAL に起因する Err はエンコーダ側の異常として `?` で上位に伝播し、SRT 接続を打ち切る fail-fast 方針とした（同関数の上の NAL 走査が既に `?` で破損 NAL を伝播している既存設計と一貫）。
- 初回 IDR 到達まで P フレーム等を捨てるためのゲート（`last_video_sample_entry.is_none()` で `Ok(None)`）は維持する。`TsSample::Video` 構築箇所では `sample_entry: self.last_video_sample_entry.clone()` を載せる。

### 不変条件 docstring の更新

- `src/video.rs` の `VideoFrame.sample_entry` docstring から「srt の Annex-B 映像」相当の経路例外記述を削除した。
- 既存負債清算として、`src/video.rs` / `src/audio.rs` の不変条件 docstring に残っていた `issue 0031` / `issue 0032` 等の issue 番号参照を削除し、経路名のみの記述に整理した。

### テスト

- 新規単体テストを `src/srt/inbound_endpoint.rs` の `mod tests` に追加した。
  - 正常系: SPS + PPS + IDR 含有 PES と P フレーム PES を順に投入して全フレームに sample_entry が載ること（`changed_since=false` で等価性も検証）
  - fail-fast: SPS / PPS 不在 IDR、SPS のみ含む IDR、PPS のみ含む IDR のいずれも `build_video_sample` が `Err` を返すこと
  - mid-stream 更新: SPS バイト列の差分で `last_video_sample_entry` が新値に上書きされて新 IDR 自身に載る挙動
  - IDR 後置 SPS / PPS: `[IDR, SPS, PPS]` 並びでも sample_entry が確定する挙動（`h264_sample_entry_from_annexb` が PES データ全体を走査することの回帰防止）
- テストヘルパとフィクスチャ:
  - `make_pending_pes(stream_id, data, pts_ticks)` を共通ヘルパとして導入し、`make_aac_pending_pes` / `make_h264_pending_pes` をその薄いラッパに整理した。
  - 映像用フィクスチャ定数 (`SPS_INITIAL` / `SPS_UPDATED` / `PPS` / `IDR` / `P_FRAME`) を `mod tests` 直下に置き、`H264AnnexBNalUnits` の検査仕様（forbidden_zero_bit のみ）を満たすバイト列とした。
  - PES 連結はテスト側で `[..].concat()` / `.to_vec()` を直接使う形に整理し、`assemble_annexb` の薄いラッパは削除した。

### レビュー指摘の反映

`/review-diff-code` で挙がった指摘を順次対応した。

- テストコメントから「設計方針 3 (3)」「設計方針 4」の節番号参照を削除し、動作仕様で説明する形に書き換えた。
- `build_video_sample` の NAL 走査コメントを実態に合わせ、`h264_sample_entry_from_annexb` の内部仕様を呼び出し側で重複説明していた部分を簡素化した。
- 当初は SPS / PPS 不在 IDR に対して `tracing::warn!` を出して破棄する設計だったが、ログ量爆発の懸念と、Err 内容（`missing H.264 SPS` / `missing H.264 PPS` / 破損 NAL）を `_` で捨てる弱さの指摘を受けて fail-fast（`?` で Err 伝播）方針に切り替えた。同関数の上の NAL 走査が既に `?` で破損 NAL を伝播している既存設計とも一貫する。設計方針 3 と完了条件、対応するテスト群もこれに合わせて更新した。

### スコープ外として後続に委ねた項目

- **`build_video_sample` の責務分離**: 70 行強の関数を `refresh_h264_sample_entry` と `assemble_video_frame` に分離する案があったが、issue 0032 (RTSP Annex-B) 未着手の現時点では共通化の正解形が見えず、YAGNI 違反のリスクが高いため据え置いた。0032 着手時に重複が現実化したタイミングで抽出を検討する。
- **`timestamp.as_micros() as u64` の `try_from` 化**: silent truncation の懸念があるが、リポジトリ内で同パターンが 22 箇所で慣用的に使われており、SRT 経路だけ修正すると整合性が崩れる。一括リファクタは別 issue で扱う。
- **AAC と H.264 の確定キー保持パターン非対称**: AAC は `(config_key, sample_entry)` の差分検出、H.264 は無条件上書き。writer 入口 muxer 側で `SampleEntry::PartialEq` の重複判定が走るため重複登録は起きず、現状の設計判断を容認した。

### CHANGES.md

記載なし（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 / 0030 と同方針。obsws 配線では subscriber 出力は必ず `VideoDecoder` を経由するため、利用者から見える挙動は変わらない。
