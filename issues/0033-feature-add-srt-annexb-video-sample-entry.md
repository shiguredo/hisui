# SRT inbound endpoint で Annex-B 映像から SPS/PPS を抽出して sample_entry を構築する

- Priority: Low
- Created: 2026-06-10
- Completed:
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

1. `H264AnnexBNalUnits::new(&pending.data)` で全 NAL を走査して `has_idr: bool` を立てる（IDR 検出時 `break` を取り除き、IDR より後ろに SPS / PPS が並ぶエンコーダ実装にも対応する）。SPS / PPS の有無は **判定しない**（`h264_sample_entry_from_annexb` の `Ok` / `Err` で代替する）
2. `has_idr` のとき `h264_sample_entry_from_annexb(0, 0, &pending.data)` を呼ぶ。`Ok(entry)` なら `self.last_video_sample_entry = Some(SharedSampleEntry::new(entry))` で更新する。`Err(_)` なら下記 3 で扱う
3. `Err(_)` パスは SPS / PPS の片方または両方が不在を意味する。`last_video_sample_entry` が `None`（確定前）のときだけ `tracing::warn!` を出し、`last_video_sample_entry` は更新しない。`Some`（確定後）の場合は warn を出さず旧 entry を維持する。フレーム自体の破棄 / 通過は本ステップでは行わず、設計方針 2 のゲートと設計方針 4 の `clone()` が処理する
4. `has_idr` が false の PES（P フレームのみ）は sample_entry 構築試行をスキップし、`last_video_sample_entry` は変更しない

width / height は 0 で構築（既存 RTMP / openh264 経路も同じ）。SPS 内 Exp-Golomb 解像度抽出は本 issue ではスコープ外。

`tracing::warn!` フォーマットは 0034 と同じ `frame_format` / `timestamp_us` キー基底に、SRT 経路固有の追加キー `reason` を載せる（0034 の writer 違反検知ログには `reason` キーは無いが、SRT 経路では「不在違反」と将来の別系統違反を区別する用途で追加する）:

```rust
tracing::warn!(
    frame_format = ?crate::video::VideoFormat::H264AnnexB,
    timestamp_us = timestamp.as_micros() as u64,
    reason = "missing_sps_pps",
    "srt_inbound_endpoint h264 frame without sample_entry; dropping until SPS-bearing IDR arrives"
);
```

`frame_format` を `?frame.format` 動的指定ではなくリテラル指定にする理由: 本関数は H.264 Annex-B PES 専用のため値が常に `VideoFormat::H264AnnexB` 固定で、frame 構築前のステップから warn を出す都合上リテラル指定で問題ない。

`timestamp` は `video_timestamp_mapper.map(dts.as_u64())` で取得した `std::time::Duration`（既存実装が `:932` で同様に得ている値）。`dts: mpeg2ts::time::Timestamp` 自体には `.as_micros()` は無く `.as_u64()`（90 kHz tick）しか無いため、必ず mapper を経由した `Duration` をログに使う。実装上は `let timestamp = self.video_timestamp_mapper.map(dts.as_u64());` を `h264_sample_entry_from_annexb` 呼び出しの前で確定させ、warn / `TsSample::Video` 構築の両方で同じ値を使う。

`reason` キーは確定前の不在違反 1 系統に絞る。`h264_sample_entry_from_annexb` の Err 文字列（SPS 欠落 / PPS 欠落 / 破損 NAL）の切り分けは本 issue では行わない（SRT Annex-B 経路で片方欠落が現実に起こる頻度は極めて低く、必要になった時点で `reason` の値を増やせばよい）。

レートリミットは入れない（0034 と同方針）。

### 4. 全フレーム付与

`TsSample::Video(crate::VideoFrame { ... })` 構築箇所（`:934-941`）で `sample_entry: None` を以下に置き換える:

```rust
sample_entry: self.last_video_sample_entry.clone(),
```

設計方針 2 のゲートにより `last_video_sample_entry` が `None` の間はそもそも下流に流れないため、ここに到達した時点で `Some` が確定している。`build_audio_samples` 内の `let sample_entry = self.last_aac_sample_entry.clone();`（実コード `:1008` 付近）と同形に揃える。`.expect(...)` 等の `Option` 解体は使わない。

### 5. mid-stream の挙動

本節は同一接続内の mid-stream を指す。再接続時の挙動は設計方針 6 を参照。

確定後の SPS / PPS 不在 IDR は設計方針 3 で `last_video_sample_entry` を更新せず、設計方針 4 で `clone()` するため旧 entry が載って下流に流れる。

新 IDR が SPS / PPS を含む場合、設計方針 3 のフローで `last_video_sample_entry` が新値（無条件 `SharedSampleEntry::new(...)` で新 Arc）に上書きされる。新 IDR フレーム自身に載せる sample_entry は更新後の新値。openh264 エンコーダ（`src/encoder/openh264.rs:55-67`）の挙動と同順序。

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
- 確定前（`last_video_sample_entry` が `None`）に SPS / PPS 不在 IDR を受信した場合は `tracing::warn!` ログを出し、当該フレームを `Ok(None)` で破棄して `last_video_sample_entry` を更新しないこと
- 確定後の mid-stream で SPS / PPS 含有 IDR が来た場合は `last_video_sample_entry` が新値に上書きされ、当該 IDR 自身に新値が載って下流に流れること（確定後の SPS / PPS 不在 IDR で旧 entry を載せて流す挙動はテスト (d) で担保する）
- 既存 `received_video_keyframe` フィールドが削除され、`last_video_sample_entry.is_some()` が唯一のゲートになっていること
- `src/video.rs:51-57` の `VideoFrame.sample_entry` docstring を本 issue マージ時点の HEAD で Read し、`、srt の Annex-B 映像（issue 0033）` 相当の記述のみを diff として削除すること（0032 の並行進行で `rtsp /` 部分が先に削られている場合があるため、HEAD の現状を確認した上で SRT 部分のみを対象にする）
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` / `video_toolbox` を含む）
- 既存 SRT 関連 e2e テスト（`e2e-tests/obsws/test_output.py` の SRT 録画系）が通ること

### CHANGES.md

記載しない（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 / 0030 と同方針。

### テスト

新規単体テストを `src/srt/inbound_endpoint.rs` の `#[cfg(test)] mod tests` に追加する。既存 AAC 音声テスト（`srt_aac_emits_sample_entry_on_every_au_with_constant_config` / `srt_aac_updates_sample_entry_on_config_change`、`:1189-1257`）と同パターンで `build_video_sample` を直接呼ぶ。

PBT は本 issue では追加しない。`SrtTsDemuxer::build_video_sample` の状態空間（`last_video_sample_entry` の有無 × IDR 有無 × SPS / PPS 有無）は単体テスト 5 ケースで網羅可能で、`h264_sample_entry_from_annexb` 内部のパース挙動は同関数の既存テスト範囲のため。

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

(b) `srt_h264_drops_idr_without_sps_pps_before_first_sample_entry`: `IDR` のみで SPS / PPS を含まない 1 PES を投入し、`build_video_sample` が `Ok(None)` を返して `last_video_sample_entry` が `None` のまま維持されることを検証。続けて (a) 相当の `SPS_A + PPS + IDR` PES を投入して確定し、以後の `P_FRAME` に sample_entry が載るまで検証

(c) `srt_h264_updates_sample_entry_on_mid_stream_sps_change`: `SPS_A + PPS + IDR` で初期確定したあと、`SPS_B + PPS + IDR` を投入し、新 IDR の sample_entry が初回と異なる（`changed_since(Some(&first)) == true`）こと、当該 IDR 自身に新 entry が載っていることを検証

(d) `srt_h264_preserves_last_sample_entry_when_subsequent_idr_lacks_sps_pps`: `SPS_A + PPS + IDR` で確定後、SPS / PPS を含まない IDR を投入し、当該 IDR が破棄されず旧 sample_entry を載せて下流に流れることを検証

(e) `srt_h264_emits_no_frame_during_consecutive_sps_pps_missing_idrs`: SPS / PPS 不在 IDR を連続 3 回投入し、いずれも `Ok(None)` で破棄され、`last_video_sample_entry` が `None` のまま維持されることを検証

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
