# RTSP subscriber に sprop-parameter-sets 解析を追加して Annex-B 映像 sample_entry を構築する

- Priority: Low
- Created: 2026-06-10
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/add-rtsp-annexb-video-sample-entry
- Polished: 2026-06-16

## 目的

RTSP subscriber の H.264 Annex-B 映像経路に対して、SDP の fmtp 行から `sprop-parameter-sets` を抽出する初期化経路と、IDR 内 inline で含まれる SPS / PPS から抽出する mid-stream 経路の 2 つを追加し、`SharedSampleEntry` を構築して `H264RtpDepacketizer` が出力する全 H.264 映像フレームに付与する。

これにより `src/video.rs` の `VideoFrame.sample_entry` docstring に残る経路例外（`rtsp の Annex-B 映像`）が削減され、不変条件が RTSP Annex-B 経路にも拡張される。

## 優先度根拠

Low。本 issue は予防的整備（broken window 解消）。現状は二重防御により `sample_entry: None` が muxer 不整合を起こす経路は無い:

- 入力側: `src/rtsp/subscriber.rs:102-114` で `want_video`（= `output_video_track_id.is_some()`）の時に `VideoDecoder` が強制生成され、subscriber 出力は decoder を経由して I420 raw へ変換される
- writer 側: 4 writer 入口の `resolve_video_sample_entry`（0034 で導入）が `sample_entry: None` を warn + fallback / skip で吸収する

不変条件 docstring に経路例外を残し続けると将来 obsws 配線が subscriber → writer 直結に変わったときの予防が効かないため対応する。

## カテゴリ判定

ブランチ `feature/add-rtsp-annexb-video-sample-entry`（`add` カテゴリ）。RTSP には `received_video_keyframe` 相当の削除対象フィールドが存在せず、不可分の整理は含まない。

## 現状

行番号は HEAD（develop = 98f6c37f）時点。実装着手時は grep で再特定する。

`src/rtsp/subscriber.rs:639` で映像フレームは `sample_entry: None` 固定で生成される（`H264RtpDepacketizer` 出力をそのまま `VideoFrame` に詰める）。`VideoFrame` 構築箇所は `:633-640`。stats 呼び出しは `:641-642` で `VideoFrame` 構築の直後。

SDP の fmtp パース状況:

- `find_fmtp`（`:1396-1409`）と `parse_fmtp_parameters`（`:1411-1423`）は既存ヘルパで、現状は音声経路 `select_audio_track`（`:1285-1345`）の `:1304` / `:1306` でのみ使用される
- 映像経路 `select_video_track`（`:1259-1283`）は payload type と clock rate のみ抽出し、fmtp は未参照
- `parse_fmtp_parameters` はキーのみを `to_ascii_lowercase()` で正規化する（`:1419`）。`sprop-parameter-sets` は元から小文字なので `params.get("sprop-parameter-sets")` でマッチする
- `find_fmtp` は値部分の文字列（空も含む）を返す。`parse_fmtp_parameters` は `;` 区切りで `key=value` ペアにパースする。`split_once('=')` で値部分は空文字列 `""` でも `Some("")` として登録される

`VideoTrackConfig`（`:245-249`）と `VideoRtpReceiver`（`:300-305`）には sample_entry を保持するフィールドが存在しない。音声側 `AudioTrackConfig`（`:252-262`）は `sample_entry: SampleEntry`（`:258`）、`AudioRtpReceiver`（`:308-318`）は `sample_entry: SharedSampleEntry`（`:317`）を持つ。

参照ヘルパ:

- `src/video/h264.rs:87-129` の `h264_sample_entry_from_annexb(width, height, data)` は戻り値 `crate::Result<SampleEntry>`。`H264AnnexBNalUnits` で `data` を走査し、SPS / PPS のいずれかが空なら `Err("missing H.264 SPS")` / `Err("missing H.264 PPS")` を返す。両方揃っていれば `SampleEntry::Avc1` を返す
- `H264AnnexBNalUnits` の `impl`（`src/video/h264.rs:28-71`）は start code prefix（`0x00 0x00 0x01` または `0x00 0x00 0x00 0x01`）と NAL header の `forbidden_zero_bit` のみ検査する。SPS / PPS の中身（Exp-Golomb 等）はパースしない
- `H264AnnexBNalUnits.next_nal_unit` は (a) start code 不在で `Err("no H.264 start code prefix")`、(b) start code 直後が空（= データ末尾の start code）で `Err("empty H.264 NAL unit")`、(c) NAL header の `forbidden_zero_bit` が立っていれば `Err("invalid H.264 NAL header: forbidden_zero_bit is set")` を返す。データ途中で `[start, start, ...]` のように 2 連続 start code が来る場合は **Err にならず**、`ty=0`（unspecified）の空 NAL を 1 個返してから次の NAL に進む
- `SharedSampleEntry`（`src/sample_entry.rs:23-67`）は `ptr_eq(&self, other) -> bool` で Arc 同一性、`changed_since(&self, prev) -> bool` で値変化を判定する。`changed_since` は別 Arc 同士でも実体比較が同値なら `false` を返すため、「同じ Arc を共有しているか」の判定には `ptr_eq` を使う必要がある
- 既存 `H264RtpDepacketizer.push_packet`（`:918-996`）は同 RTP timestamp の複数 packet を marker=true まで `current_data` に Annex-B 形式で連結し続け、marker=true で `take_frame` する。Single NAL 連続 / STAP-A / FU-A いずれの経路でも、IDR と同 timestamp 内に SPS / PPS が来て中間 marker=true が挟まらなければ `frame.data` に SPS / PPS / IDR が Annex-B で揃う

`crate::Error` 周辺（`src/error.rs:1-90`）:

- `Error::new(reason)` は `#[track_caller]` で `Location::caller()` を取り `Backtrace::capture()` する。`crate::Error` を `e.to_string()` で文字列化して `Error::new` で再ラップすると `location` が呼び出し側に上書きされる
- `impl<E: std::fmt::Display> From<E> for Error` は実装しない方針（`error.rs:75-79`）。`crate::Error` の `?` 伝播は `crate::Result` 内では素通し、外部クレートの Err（`base64ct::Error` 等）は `.map_err(|e| Error::new(format!("...: {e}")))?` で `crate::Error` にラップする
- `crate::Error` を `SessionError::Fatal` に乗せ替えるときは `.map_err(SessionError::Fatal)?`（`subscriber.rs:419 / 457 / 491 / 628 / 650 / 653` で多数の前例）

依存クレート: Base64 デコードは既存 `base64ct` を使う（`Cargo.toml:54` で `features = ["alloc"]`、`src/obsws/auth.rs:1` に `encode_string` の利用例）。`Base64::decode_vec(input: &str) -> Result<Vec<u8>, base64ct::Error>` は RFC 4648 §4 の padding 厳格モードで、RFC 6184 §8.1 の `sprop-parameter-sets` 規定と整合する。

## 設計方針

### 1. `VideoTrackConfig` への sample_entry 保持フィールド追加

`VideoTrackConfig`（`:245-249`）に `sample_entry: Option<SampleEntry>` フィールドを追加する。`Option` とするのは SDP に `sprop-parameter-sets` が含まれない構成（RFC 6184 §8.2.1 で MAY）を許容するため。型は既存 `AudioTrackConfig.sample_entry: SampleEntry` の対称性で生 `SampleEntry`。`SharedSampleEntry` ラップは `VideoRtpReceiver` 初期化時に行う。

音声側 `select_audio_track` は fmtp 不在で `Err` を返すが、映像側で `Err` にしないのは inline SPS / PPS による代替経路があるため。

### 2. `select_video_track` での SDP fmtp パース追加

`select_video_track`（`:1259-1283`）の `Ok(Some(VideoTrackConfig { ... }))` 構築直前で以下を行う:

1. `find_fmtp(&media.attributes, payload_type)` を呼ぶ。`None` の場合は `sample_entry: None` で `VideoTrackConfig` を返す
2. `parse_fmtp_parameters(&fmtp)` で `HashMap<String, String>` を取得する
3. `params.get("sprop-parameter-sets").map(String::as_str)` で `Option<&str>` を得る。`None` か `Some("")`（空文字列）の場合は `sample_entry: None` で返す
4. 値を `,` で分割し、各要素を `.trim()` する。空要素（連続 `,,` や前後 `,` 由来）はスキップする
5. 各 trim 後の要素を `base64ct::Base64::decode_vec` で Base64 デコードする。`Err` は `.map_err(|e| crate::Error::new(format!("invalid sprop-parameter-sets base64: {e}")))?` で `crate::Error` にラップする
6. デコード結果の各 NAL ユニットの前に `[0x00, 0x00, 0x00, 0x01]` を挿入して連結し Annex-B バイト列を生成する
7. `crate::video::h264::h264_sample_entry_from_annexb(0, 0, &annexb)` を呼ぶ。失敗は `?` で `crate::Error` のまま素通しする
8. 構築結果を `VideoTrackConfig.sample_entry` に格納する

width / height は 0 で渡す。`h264_sample_entry_from_annexb` は引数の width / height を `Avc1Box.visual` にそのまま埋めるだけで SPS パースはしない。SPS 内 Exp-Golomb 解像度抽出は本 issue ではスコープ外。

エラー方針:

- Base64 デコード失敗は `.map_err` で `crate::Error` に変換、それ以外（`h264_sample_entry_from_annexb` の `Err`）は `?` で素通し
- 呼び出しチェーン `select_tracks`（`:1233`）→ `setup_session`（`:419`）の `select_tracks(...).map_err(SessionError::Fatal)?` で `SessionError::Fatal` に変換される
- `SessionError::Fatal` は `run`（`:151-162`）の match で `return Err(e)` され、`RtspSubscriber::run` 自体が異常終了する（再接続しない）。SDP の不正値はサーバ設定ミスとして処理を中断する設計。Retryable に倒すと SDP は接続ごとに同じ値が返るため無限ループになるリスクがある

fmtp 不在・`sprop-parameter-sets` 不在・空文字列値・空要素のみ（`sprop-parameter-sets=,,`）は `Err` にしない。後段の inline 経路で確定する。

### 3. `VideoRtpReceiver` への sample_entry 保持フィールド追加

`VideoRtpReceiver`（`:300-305`）に `last_sample_entry: Option<SharedSampleEntry>` フィールドを追加する。`setup_session` 関数（`:391-508`）内の video 分岐ブロック（`:425-460`）で `self.video_receiver = Some(VideoRtpReceiver { ... })` 構築（`:449-459`）時に `video.sample_entry.map(SharedSampleEntry::new)` で初期化する。

フィールド名 `last_sample_entry` は `Openh264Encoder.last_sample_entry`（`src/encoder/openh264.rs:15`）と同形（mid-stream 更新を示す `last_` プレフィックス）。`VideoRtpReceiver` は映像専用構造体のため `video` 接頭辞は付けない。

### 4. `apply_video_frame_sample_entry` 純関数とゲートの導入

新規 free function を `src/rtsp/subscriber.rs` の `H264RtpDepacketizer` 定義（`:907-`）の近くまたは `handle_rtp_packet` の上に追加する（RTP データ処理系の役割上の近さを優先）。`use` 文として `crate::video::h264::{H264AnnexBNalUnits, H264_NALU_TYPE_IDR, H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS, h264_sample_entry_from_annexb}` をファイル先頭に追加する。

```rust
fn apply_video_frame_sample_entry(
    receiver: &mut VideoRtpReceiver,
    frame: &DepacketizedVideoFrame,
) -> crate::Result<()> {
    // NAL 走査で has_idr / has_sps / has_pps を判定する。
    // NAL 走査自身が Err なら crate::Error を素通しする。
    let mut has_idr = false;
    let mut has_sps = false;
    let mut has_pps = false;
    for nalu in H264AnnexBNalUnits::new(&frame.data) {
        let nalu = nalu?;
        match nalu.ty {
            H264_NALU_TYPE_IDR => has_idr = true,
            H264_NALU_TYPE_SPS => has_sps = true,
            H264_NALU_TYPE_PPS => has_pps = true,
            _ => {}
        }
    }

    // IDR + SPS + PPS の 3 条件揃いの IDR でのみ sample_entry を更新する。
    // 3 条件揃わない IDR は更新試行をスキップし Err にしない。
    if has_idr && has_sps && has_pps {
        let entry = h264_sample_entry_from_annexb(0, 0, &frame.data)?;
        receiver.last_sample_entry = Some(SharedSampleEntry::new(entry));
    }

    Ok(())
}
```

`handle_rtp_packet`（`:614-`）の VIDEO 分岐（`:621-663`）の `for frame in frames` ループ（`:629-661`）内を以下の順に組み替える:

```rust
for frame in frames {
    let timestamp = video_receiver.timestamp_mapper.map(u64::from(frame.rtp_timestamp));

    apply_video_frame_sample_entry(video_receiver, &frame)
        .map_err(SessionError::Fatal)?;

    let Some(sample_entry) = video_receiver.last_sample_entry.clone() else {
        continue;
    };

    let video_frame = VideoFrame {
        data: frame.data,
        format: VideoFormat::H264AnnexB,
        keyframe: frame.keyframe,
        size: None,
        timestamp,
        sample_entry: Some(sample_entry),
    };
    stats.add_input_video_frame_count();
    stats.set_last_input_video_timestamp(timestamp);

    if let Some(decoder) = output.video_decoder.as_mut()
        && let Some(tx) = output.video_track_tx.as_mut()
    {
        // 既存 :646-660 の処理をそのまま流用する
        // （`Arc::new(video_frame)` で包んで `decoder.handle_input_sample` + `drain_video_decoder_output`）
    }
}
```

設計判断の根拠:

- (a) 3 条件判定するのは、SDP `sprop-parameter-sets` で初期確定済みの RTSP カメラが IDR には SPS / PPS を inline しない実装が多いため。`has_idr` のみで `h264_sample_entry_from_annexb` を呼ぶと、これらの環境で初回 IDR ごとに `Err("missing H.264 SPS")` で接続が打ち切られる。SRT 0033 は MPEG-TS の publisher 側で SPS / PPS inline が標準のため `has_idr` だけで判定したが、RTSP は入口が異なる
- (b) fail-fast の対象は (i) NAL 走査自身の `Err`（start code 不在 / 空 NAL / forbidden_zero_bit 設定）と (ii) 3 条件揃った IDR で `h264_sample_entry_from_annexb` がパース失敗した場合の 2 種類。どちらも publisher 側エンコーダの異常または伝送破損で接続を打ち切る
- (c) mid-stream で SPS / PPS の片方だけが inline されてくる IDR（SPS のみ / PPS のみ / 両方なし）は 3 サブケースとも `has_sps && has_pps` のショートサーキットで同じ「更新スキップ + `Ok(())`」経路に流れる。サブケース個別のテストは (j) で代表 1 ケース（両方不在 IDR）を検証する
- (d) `let-else` でゲートを書くのは、`continue` 後の `VideoFrame.sample_entry: Some(...)` で `unwrap` / `expect` を避けるため。stats 呼び出しはゲート通過後 / `VideoFrame` 構築後の現コード `:641-642` と同位置に置く。stats の意味論（下流に流れた frame 数）は現状を維持し、ゲートで破棄される frame は stats に乗らない

`apply_video_frame_sample_entry` の責務範囲は NAL 走査 + 3 条件判定 + 必要時の `h264_sample_entry_from_annexb` 呼び出し + `receiver.last_sample_entry` 更新のみ。timestamp 計算、ゲート、`VideoFrame` 構築、stats 更新、decoder 投入は呼び元に残す。戻り値型は `crate::Result<()>` で、`SessionError::Fatal` 変換は呼び元で一度だけ行う。

二重走査について: `apply_video_frame_sample_entry` の走査と `h264_sample_entry_from_annexb` の内部走査で同じ `frame.data` を 2 度走査する。SRT 0033 と同方針。一段化は将来の最適化として別 issue で扱う。

### 5. 周辺挙動の取り扱い

- **RTSP 切断・再接続**: `run`（`:81-167`）の外側 `loop`（`:133-166`）で `SessionError::Retryable` 時に `RtspSessionRunner` 全体が再生成され、`VideoRtpReceiver.last_sample_entry` を含む全フィールドが初期化される。再接続後は新規接続と同じ通常フロー。`SessionError::Fatal` は再接続せず `RtspSubscriber::run` 自体が `Err` で終了する
- **Re-DESCRIBE / PLAY pause-resume**: 既存 `RtspSessionRunner` は PLAY 後に DESCRIBE を呼び返さない。本 issue でも介入しない
- **同一 SPS / PPS 連続 IDR**: muxer 側 `shiguredo_mp4::mux::Mp4FileMuxer` は `SampleEntry::PartialEq` で実体比較するため、無条件上書きで新 Arc が作られても重複登録は起きない。AAC 側の `(config_key, sample_entry)` 差分検出パターンは採らない

### 6. 不変条件コメントの例外記述更新

`src/video.rs:51-57` の `VideoFrame.sample_entry` docstring の経路例外から `rtsp` 部分を削る。本 issue マージ時点の HEAD を Read で確認し、現行の `現時点で未適用の経路: WebM リーダー、rtsp の Annex-B 映像。` から `rtsp の Annex-B 映像` 相当の記述のみを diff として削除する（並行進行中の 0031 で `WebM リーダー` 部分が先に削られている場合があるため、HEAD の現状を確認した上で RTSP 部分のみを対象にする）。

`src/audio.rs:92` の `AudioFrame.sample_entry` docstring には RTSP 経路例外が無いため本 issue では触らない。

## 完了条件

- `H264RtpDepacketizer` 出力フレームが全て `Some(SharedSampleEntry)` を持つこと
- SDP `sprop-parameter-sets` がある場合は setup_session 完了時点で `last_sample_entry` が確定し、最初のフレームから下流に流れること
- SDP `sprop-parameter-sets` が無い場合は初回 SPS / PPS 揃った IDR まで全フレームが破棄され、確定後の全フレームに同じ entry が clone されて付与されること
- mid-stream で SPS / PPS 揃った IDR が来た場合は `last_sample_entry` が新値に上書きされ、当該 IDR 自身に新値が載って下流に流れること
- mid-stream の IDR に SPS / PPS が片方でも無い場合は更新試行をスキップし、既存の `last_sample_entry` を維持すること（SDP 由来確定済みの一般的 RTSP カメラで接続切断が発生しないこと）
- Base64 デコード失敗、`H264AnnexBNalUnits` の NAL 走査 Err、SPS / PPS 揃った IDR で `h264_sample_entry_from_annexb` が Err を返した場合は `SessionError::Fatal` で接続を打ち切ること
- `src/video.rs:51-57` の `VideoFrame.sample_entry` docstring から `rtsp の Annex-B 映像` 相当の記述を削除すること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること（feature gate `fdk-aac` / `nvcodec` および macOS では `shiguredo_video_toolbox` 関連を含む）

### CHANGES.md

記載しない（内部リファクタ・公開 API 変化なし・利用者挙動変化なし）。0017 / 0027 / 0030 / 0033 と同方針。

### テスト

新規単体テストを `src/rtsp/subscriber.rs` の `#[cfg(test)] mod tests`（`:1478-`）に追加する。既存テスト群（`parse_rtsp_input_url_*`、`depacketize_h264_*`、`run_rtsp_session_*`）と同階層。`VideoRtpReceiver`・`DepacketizedVideoFrame` は同モジュール内 private のため、`mod tests` から `super::` で直接アクセスする。

#### テストヘルパとフィクスチャ

`mod tests` 直下に以下を追加する:

- 映像用 Annex-B バイト列定数（SRT 0033 と同じバイト列を独立定義する。共有化はスコープ外）:
  - `SPS_INITIAL`: `[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1e, 0xab]`
  - `SPS_UPDATED`: `[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1e, 0xac]`
  - `PPS`: `[0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x06, 0xe2]`
  - `IDR`: `[0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21]`
  - `P_FRAME`: `[0x00, 0x00, 0x00, 0x01, 0x41, 0x9a, 0x21, 0x6c]`
  - `BROKEN_NAL`: `[0x00, 0x00, 0x00, 0x01, 0x85, 0x00, 0x01]`（NAL header `0x85` で `forbidden_zero_bit` が立つ）
- ヘルパ関数（VideoRtpReceiver は構造体生成の利便性のためのヘルパで、`apply_video_frame_sample_entry` は内部で `last_sample_entry` 以外のフィールドを読まないが、構造体生成上は全フィールド初期化が必要）:
  - `fn build_test_video_receiver() -> VideoRtpReceiver`: `rtp_channel: 0`, `payload_type: 96`, `timestamp_mapper: TimestampMapper::new(32, 90_000, Duration::ZERO).expect("テスト用の TimestampMapper が構築できること")`, `depacketizer: H264RtpDepacketizer::new()`, `last_sample_entry: None`
  - `fn build_test_depacketized_frame(data: Vec<u8>) -> DepacketizedVideoFrame`: `rtp_timestamp: 0`, `keyframe: false`, `data`（`apply_video_frame_sample_entry` は `frame.keyframe` を読まないため、`keyframe` 値は検証に影響しない）
  - `fn build_test_sdp_with_fmtp(fmtp_params: &str) -> String`: 既存 `build_test_sdp(false, false)`（`:2075-2100`）ベースで `a=fmtp:96 <fmtp_params>\r\n` を `a=control:trackID=0\r\n` の直前に挿入した SDP テキストを返す。SDP の改行は `\r\n` で統一する
  - `build_test_sdp_with_sprop` は別ヘルパとしては作らず、テスト内で `build_test_sdp_with_fmtp(&format!("sprop-parameter-sets={sprop_value}"))` を直接書く

Base64 エンコード用に `base64ct::Base64::encode_string` を使い、SPS / PPS バイト列の start code prefix 4 バイトを除いた素の NAL バイト列から `sprop-parameter-sets` 値を組み立てる。

#### テストケース

`select_video_track` 単体（SDP fmtp パース経路）:

- (a) `select_video_track_extracts_sample_entry_from_sprop`: `SPS_INITIAL` と `PPS` の Base64 連結値を含む SDP に対して `select_video_track` を呼び、`VideoTrackConfig.sample_entry` が `Some(SampleEntry::Avc1(_))` で返り、`avcc_box.sps_list` / `pps_list` が入力 NAL バイト列（start code 除く）と一致することを検証
- (b) `select_video_track_returns_none_when_fmtp_missing`: 既存 `build_test_sdp(false, false)`（fmtp 不在）で `VideoTrackConfig.sample_entry: None`
- (c) `select_video_track_returns_none_when_sprop_missing_in_fmtp`: `build_test_sdp_with_fmtp("profile-level-id=42c01e;packetization-mode=1")` で `VideoTrackConfig.sample_entry: None`
- (d) `select_video_track_returns_none_when_sprop_empty`: `sprop-parameter-sets=`（空文字列値）で `VideoTrackConfig.sample_entry: None`
- (e) `select_video_track_returns_none_when_sprop_has_only_empty_entries`: `sprop-parameter-sets=,,`（カンマのみで空要素のみ）で `VideoTrackConfig.sample_entry: None`（trim 後にスキップされ後段の Annex-B 連結に到達しない）
- (f) `select_video_track_returns_err_on_invalid_base64`: `sprop-parameter-sets=!!!`（Base64 アルファベット外）で `Err` を返す（`base64ct::Error` が `crate::Error` にラップされる）
- (g) `select_video_track_returns_err_on_sprop_with_only_sps`: SPS のみの sprop で `Err("missing H.264 PPS")` を伝播
- (h) `select_video_track_returns_err_on_sprop_with_only_pps`: PPS のみの sprop で `Err("missing H.264 SPS")` を伝播

`apply_video_frame_sample_entry` 単体（depacketizer 出力経路）:

- (i) `apply_video_frame_sample_entry_emits_sample_entry_for_sps_pps_idr_frame`: `last_sample_entry: None` 初期状態で `SPS_INITIAL + PPS + IDR` 連結の `DepacketizedVideoFrame` を渡し、`Ok(())` が返り `receiver.last_sample_entry` が `Some` になることを検証
- (j) `apply_video_frame_sample_entry_keeps_initial_sample_entry_for_mid_stream_idr_without_sps_pps`: (i) と同じ手順で初期確定後（`initial = receiver.last_sample_entry.clone().unwrap()`）、`IDR` 単体（SPS / PPS 不在）frame を渡し、`Ok(())` が返り `receiver.last_sample_entry.as_ref().unwrap().ptr_eq(&initial)` で同一 Arc を共有していることを検証
- (k) `apply_video_frame_sample_entry_updates_sample_entry_on_mid_stream_sps_change`: (i) と同じ手順で初期確定後、`SPS_UPDATED + PPS + IDR` を渡し、`receiver.last_sample_entry.as_ref().unwrap().changed_since(Some(&initial)) == true` で値変化を検証し、さらに `!receiver.last_sample_entry.as_ref().unwrap().ptr_eq(&initial)` で別 Arc であることを検証
- (l) `apply_video_frame_sample_entry_skips_update_for_p_frame_only`: `last_sample_entry: None` 初期状態で `P_FRAME` 単体を渡し、`Ok(())` が返り `last_sample_entry` が `None` のまま不変であることを検証
- (m) `apply_video_frame_sample_entry_returns_err_on_broken_nal`: `BROKEN_NAL` 単体を渡し、NAL 走査 Err（`invalid H.264 NAL header: forbidden_zero_bit is set`）が `?` で `Err` として返ることを検証

(i) と (l) の差: (i) は IDR + SPS + PPS の 3 条件揃った frame で更新が走る正常系、(l) は P フレームのみで NAL 走査自体は成功するが `has_idr=false` で更新分岐に入らない非更新系。両者で `has_idr` 単独ゲートと `has_idr && has_sps && has_pps` ゲートを取り違える回帰を検出できる。

(j) は SDP `sprop-parameter-sets` 由来で確定済みの一般的 RTSP カメラ実装（mid-stream IDR に SPS / PPS を inline しない）の挙動を fail-fast にしないことの回帰防止。設計方針 4 (a) の 3 条件判定の本質的テスト。SPS のみ inline / PPS のみ inline の 2 サブケースは `has_sps && has_pps` のショートサーキットで (j) と同じ経路を踏むため個別テストは追加しない（設計判断根拠 (c) 参照）。

PBT は追加しない。状態空間は単体テスト 13 ケースで網羅可能。

既存テスト `run_rtsp_session_*` 系（`:1599-1720`）への影響:

- 既存 3 テストは `RtspOutputContext` に `video_decoder: &mut None` を渡す（`:1618-1619` 他）ため、映像 frame は decoder 投入分岐に入らない。ゲート挿入後も挙動は変わらない
- `run_rtsp_session_disconnects_after_requesting_audio_and_video` 内の `send_test_video_rtp`（`:2102-2114`）は SPS / PPS なし IDR Single NAL を送るが、新仕様では `has_sps && has_pps` 不成立で `h264_sample_entry_from_annexb` を呼ばず、`last_sample_entry: None` のままゲートで破棄される。`assert!(result.is_err(), ...)` は PLAY レスポンス後の disconnect で検証する元の意図のままで通る
- 既存テストの payload や SDP の sprop 追加は行わない

### 影響範囲確認

実装着手前と完了時に以下を grep する:

- `rg 'sample_entry:\s*None' src/rtsp/subscriber.rs`: 着手前は `:639` で 1 件、完了時は 0 件
- `rg 'last_sample_entry' src/rtsp/subscriber.rs`: 着手前は 0 件、完了時は構造体定義 / 初期化 / 更新サイト / 参照サイト / テスト群で 15 件以上
- `rg 'sprop-parameter-sets' src/rtsp/subscriber.rs`: 着手前は 0 件、完了時は `select_video_track` 内とテスト群で 5 件以上
- `rg 'find_fmtp|parse_fmtp_parameters' src/rtsp/subscriber.rs`: 着手前は定義 2 件 + 音声経路 `:1304` / `:1306` の 2 件呼び出しの計 4 件、完了時は映像経路の 2 件呼び出し追加で計 6 件
- `rg 'apply_video_frame_sample_entry' src/rtsp/subscriber.rs`: 着手前は 0 件、完了時は定義 1 件 + `handle_rtp_packet` からの呼び出し 1 件 + テスト群で 10 件以上
- `rg 'resolve_video_sample_entry' src/`: 計 9 件で着手前と完了時で件数が変わらない（本 issue では writer 側を変更しないため）

## スコープ外

- **SPS 内 Exp-Golomb 解像度抽出**: `Avc1Box.visual.width/height` を 0 のまま埋める。実値抽出は RTMP / openh264 / SRT 横断で別 issue
- **SDP `profile-level-id` / `packetization-mode` 等の他 fmtp パラメータ反映**: `Avc1Box.avcc_box` の `avc_profile_indication` / `avc_level_indication` は固定値のまま。別 issue
- **Re-DESCRIBE 対応**: 既存 `RtspSessionRunner` が実装していないため本 issue でも介入しない
- **STAP-B / FU-B / MTAP16 / MTAP24 のサポート**: 既存 `H264RtpDepacketizer` がサポートしないパケット種別（`:983-987` で `Err`）
- **`AudioRtpReceiver.sample_entry` docstring / `handle_rtp_packet` 内 AAC コメントの `issue 0030` 参照削除**: `:315` と `:677` の 2 件は別途清算 issue で扱う
- **sprop 由来と inline 由来の差分検出ログ**: SDP 由来確定後に異なる SPS / PPS が来た場合の警告ログは本 issue では出さない
- **テストフィクスチャ共有化**: SRT 0033 と RTSP 0032 で同じ SPS / PPS / IDR / P_FRAME 定数を持つことになるが、共有モジュール化は 0033 側の改変を伴うため別 refactor issue で扱う
- **永久破棄の検知・回復**: SDP に `sprop-parameter-sets` が無く、かつ mid-stream の IDR にも SPS / PPS が一度も inline されない publisher（業界実態では稀だが理論的にあり得る）では `last_sample_entry` が永久に `None` のまま全フレームが破棄される。タイムアウトや警告ログによる検知は本 issue では設けない
- **Base64 padding 省略 publisher のサポート**: 別 issue で扱う

## 関連

- issue 0030（直接の前提。リーダー / AAC 音声入力経路への不変条件適用と writer 補完削除。closed）
- issue 0033（SRT inbound endpoint の Annex-B 映像 sample_entry 構築。本 issue の設計の雛形。closed）
- issue 0034（writer 入口の `resolve_video_sample_entry` 違反検知 + fallback。本 issue 完了で RTSP Annex-B 経路からの違反流入が構造的に消える。closed）
- issue 0027（映像エンコーダの全フレーム付与と `VideoFrame.sample_entry` の `SharedSampleEntry` 化。間接的な前提。closed）
- issue 0017（音声側の `SharedSampleEntry` 共通型導入。間接的な前提。closed）
- issue 0031（WebM リーダーへの sample_entry 構築追加。本 issue と並行・独立で進める。`src/video.rs` の不変条件コメント編集はマージ順序により互いに影響する）

## 解決方法

実装着手後にここに記述する。
