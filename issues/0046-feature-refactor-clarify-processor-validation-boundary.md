# 5 processor 構造体の validation 責務分担を確定して暴露面を塞ぐ

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-clarify-processor-validation-boundary
- Polished:

## 目的

closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`) で `RtmpInboundEndpoint` / `RtmpOutboundEndpoint` / `RtmpPublisher` / `SrtInboundEndpoint` / `RtspSubscriber` の `impl TryFrom<nojson::RawJsonValue>` が削除され、そこに集約されていた以下の不変条件 (validation) も同時にコード上から消えた。

- `stream_name must not be empty` (RTMP 3 構造体)
- `outputAudioTrackId or outputVideoTrackId is required` (RTMP inbound / SRT / RTSP)
- `inputAudioTrackId or inputVideoTrackId is required` (RTMP outbound / publisher)
- `certPath and keyPath must be specified together` (RTMP outbound)
- `rtmps://` のとき `certPath` 必須 (RTMP outbound)
- `keyLength requires passphrase` (SRT — 同等チェックは `SrtInboundEndpoint::endpoint_config` 内に残存)
- `maxBufferedFrameCount must be greater than 0` (RTMP publisher — **削除済み、再実装なし**)

5 構造体はいずれも `pub struct` + `pub` フィールド公開のままで、現在は obsws coordinator 1 経路のみが組み立てに使うため即時の事故は出ていない。しかし将来別経路 (例: state_file overlay 経由の `SetInputSettings`、別 IPC、テスト ヘルパ等) から不正値が流れた場合、

- `max_buffered_frame_count: 0` で `src/rtmp/publisher.rs:82` の `tokio::sync::mpsc::channel(0)` が `assert!` panic 確定
- 空文字 `stream_name` が `parse_rtmp_url(...)` まで届いてランタイムエラー or 想定外挙動
- `cert_path` / `key_path` の片方のみで `get_cert_and_key_paths` が `"Private key path not specified"` 等を返す経路

など、Rust 型システムから保証されない暴露面が広がっている。本 issue は **validation の責務分担 (構造体側 / coordinator 側 / 型レベル) を確定** し、選んだ方針に従って必要な改修を入れて、closed 0041 の dead-code 整理で残った設計上の隙間を塞ぐ。

## 優先度根拠

Low。

- 現状の呼び出し元 (obsws coordinator 1 経路) では即時の事故は起きていない
- 将来別経路を増やすときの落とし穴 + 設計判断の明文化のため、利用者影響ゼロのうちに設計を確定する価値あり
- 「Don't live with broken windows」原則に照らすと、消えた validation を黙って放置するのは負債化を待つ状態

## 現状

### 関連 grep ヒット (2026-06-18 時点)

- `src/rtmp/inbound_endpoint.rs` の `RtmpInboundEndpoint` フィールド: `pub input_url: String` / `pub stream_name: Option<String>` / `pub output_audio_track_id: Option<TrackId>` / `pub output_video_track_id: Option<TrackId>` / `pub options: RtmpInboundEndpointOptions`
- `src/rtmp/outbound_endpoint.rs` の `RtmpOutboundEndpoint`: `output_url` / `stream_name` / `input_audio_track_id` / `input_video_track_id` / `options.cert_path` / `options.key_path` がすべて `pub`
- `src/rtmp/publisher.rs` の `RtmpPublisher`: `output_url` / `stream_name` / `input_audio_track_id` / `input_video_track_id` / `options.max_buffered_frame_count` がすべて `pub`。`max_buffered_frame_count` は `pub usize` で `0` を弾く型レベル保証なし
- `src/srt/inbound_endpoint.rs` の `SrtInboundEndpoint`: `input_url` / `output_*_track_id` / `stream_id` / `passphrase` / `key_length` / `tsbpd_delay_ms` すべて `pub`。`endpoint_config()` (line 290 付近) で `keyLength requires passphrase` の検証は残るが、`u16::MAX` 超過 tsbpd 等は実行時エラー
- `src/rtsp/subscriber.rs` の `RtspSubscriber`: `input_url` / `output_video_track_id` / `output_audio_track_id` すべて `pub`

### obsws 経路の現状 validation

`src/obsws/state/types.rs` の `parse_optional_string_setting` (line 219 付近) は値の取り出しのみで、`stream_name` / `input_url` / `stream_id` / `passphrase` 等の **空文字検証は一切行わない**。`ObswsRtmpInboundSettings.stream_name` 等の `Option<String>` フィールドに `""` (空文字) を持つ値がそのまま endpoint 構造体に届く経路がある。

### tokio mpsc の panic 仕様

`tokio::sync::mpsc::channel(buffer)` は `buffer == 0` で `assert!(buffer > 0)` により panic 確定 (tokio 公式 docs および bounded.rs 実装で確認可能)。`RtmpPublisher` のみがこの API を直接呼んでおり、`max_buffered_frame_count: 0` が届くと無条件 panic する。

### 関連経路 (validation を入れる場合の候補)

- `src/obsws/coordinator/output_rtmp.rs:282` の `start_rtmp_outbound_processors` (RtmpOutboundEndpoint 組立)
- `src/obsws/coordinator/output_stream.rs:359` の `start_stream_processors` (RtmpPublisher 組立)
- `src/obsws/source/rtmp_inbound.rs:26` の `build_record_source_plan` (RtmpInboundEndpoint 組立)
- `src/obsws/source/srt_inbound.rs:26` (SrtInboundEndpoint 組立)
- `src/obsws/source/rtsp_subscriber.rs:26` (RtspSubscriber 組立)
- `src/obsws/state/types.rs` の `parse_optional_string_setting` (空文字検証の追加候補地)

## 設計方針

設計判断が必要な issue。`/polish-issue` で詰める前に、以下のいずれの方針を採るか実装着手者がプロジェクト全体の方針と整合させて決める。

### 案 A: 構造体側に閉じ込める (`non_exhaustive` + コンストラクタ)

各構造体を `#[non_exhaustive]` + `fn new(...) -> Result<Self, _>` パターンに変える。`pub` フィールドアクセスは `pub(crate)` に格下げ。

- 長所: 型レベルで「構造体インスタンスは validation 済み」を保証できる。呼び出し元が増えても暴露面が広がらない
- 短所: obsws coordinator 側の構造体リテラル組立 (`RtmpPublisher { output_url: ..., ... }`) を `RtmpPublisher::new(...)` 呼びに書き換える必要があり差分が広い。テストでの組立ヘルパも追従が必要

### 案 B: 型レベルで保証する (`NonZeroUsize` 等)

- `RtmpPublisherOptions::max_buffered_frame_count` を `usize` → `NonZeroUsize` に昇格
- `stream_name`, `stream_id`, `passphrase` 等を「非空保証付きの新型」(`pub struct NonEmptyString(String)` 等) に昇格

- 長所: 型システムで保証されるため Rust 的に綺麗
- 短所: 新型の導入コストと API 表面の変化。obsws 経路のパーサ (`parse_optional_string_setting`) も新型対応が必要

### 案 C: coordinator 側に validation を集約 (構造体は値運搬のみ)

`src/obsws/coordinator/` および `src/obsws/source/` 配下の組立関数で validation を行い、5 構造体は「正規化済みの値運搬」と割り切る。構造体側の `pub` は維持。

- 長所: 差分が局所的 (coordinator 関数の冒頭に検査を追加するだけ)。型変更なし
- 短所: 「将来別経路から構造体を直接組み立てたとき」の保証は型では取れず、ドキュメント / コードレビューで担保するしかない。`max_buffered_frame_count: 0` の `tokio` panic は `RtmpPublisher::run` 冒頭の defense-in-depth でしか塞げない

### 案 D: 案 C + 構造体側の `run()` 冒頭 defense-in-depth

案 C を採りつつ、`RtmpPublisher::run` / `SrtInboundEndpoint::run` 等の `run` 冒頭で「最低限の panic 防止」のみ追加 (`max_buffered_frame_count == 0` 等)。

- 長所: 案 C のシンプルさを保ちつつ、最悪の panic 経路 (`tokio::sync::mpsc::channel(0)`) は塞げる
- 短所: 二段の検査で責務分担が曖昧になる懸念

### docs / コメントの最小要件

どの案を採っても、以下を docs / コメントに明文化する:

- 5 構造体の doc コメントに「フィールドは検証済みであること (空文字でない、URL 形式、片方は Some、`max_buffered_frame_count >= 1` 等) を呼び出し側で保証する責務」または「コンストラクタを使用すること」を明記
- 検証の集約場所 (構造体 / coordinator / 型) を `docs/internals/` 配下に責務分担ノートとして残す (open issue 0040 (`プロセッサの種類別の実装規約と不変条件を docs/internals/ にまとめる`) との合流候補)

## 完了条件

- 採用方針が決定され、5 構造体の doc コメントまたは `docs/internals/` 配下に validation 責務分担が明記されていること
- `max_buffered_frame_count: 0` で `tokio::sync::mpsc::channel(0)` panic が起きない経路が型 / コンストラクタ / `run` 冒頭 / coordinator のいずれかで保証されていること
- obsws 経路の `stream_name: ""` / `input_url: ""` 等の空文字流入が、採用方針に従って「拒否 / None 化 / 明示許容」のいずれかに確定し、テストで検証されていること
- 削除前 `TryFrom` が担っていた他の不変条件 (track_id 片方必須、cert / key パスペア、`rtmps://` で `certPath` 必須、`keyLength requires passphrase`) について、採用方針に従って保証 or 明示放棄が確定していること
- 関連テスト (構造体側または coordinator 側) が追加され、`cargo test --workspace` が通ること
- `cargo fmt --all --check` / `cargo check --workspace` / `cargo check --workspace --no-default-features` / `cargo clippy --workspace --all-targets -- --deny warnings` / `cargo clippy --workspace --no-default-features -- --deny warnings` / `cargo test --workspace` がすべて通ること

## CHANGES.md について

採用方針によって扱いが変わる。実装着手時に判断する:

- 案 A / B を採って `pub struct` フィールド構造を変更する場合、`crate::rtmp::publisher::RtmpPublisher { ... }` の組立構文が変わるため crate 外の利用者には後方互換のない変更となる。ただし本 crate は library として外部公開していないため、現状 `CHANGES.md` 記載は不要 (obsws coordinator 内部のみで使われる前提)
- 案 C / D を採って coordinator 側に validation を追加する場合、obsws WebSocket 経由で「今までは通っていた空 `stream_name` が拒否される」等の利用者観点の振る舞い変化があれば `[CHANGE]` 系で記載する
- 採用方針確定時にこの節を polish-issue で改訂する

## 関連

- closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`): 本 issue 対象の不変条件が削除された直接の契機。レビューで「設計上の暴露面が残る」と指摘された分の後追い
- open issue 0040 (`プロセッサの種類別の実装規約と不変条件を docs/internals/ にまとめる`): 5 構造体の validation 責務分担を `docs/internals/` に残す場合、本 issue の成果物が 0040 の章として吸収される可能性あり。0040 と先後関係を整理する
- closed PR #207 (`feature/remove-json-rpc` / merge commit `d4170ed8`): 元々 validation が乗っていた JSON-RPC 機能の削除元
