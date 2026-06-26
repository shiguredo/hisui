# `processor` 構造体の validation 責務分担

この文書は、5 つの processor 構造体（`RtmpInboundEndpoint` / `RtmpOutboundEndpoint` / `RtmpPublisher` / `SrtInboundEndpoint` / `RtspSubscriber`）について、フィールドの不変条件をどこで保証するかをまとめます。

過去の経緯として、これら 5 構造体には `impl TryFrom<nojson::RawJsonValue>` が存在し、構造体を組み立てる際に eager に validation を実施していました。
当該実装は `feature/refactor-remove-unused-processor-json-impls`（merge commit `42979dae`）で JSON-RPC 経路廃止に伴い削除されており、削除直後はフィールドが `pub` のままで型システムから保証されない暴露面が残っていました。
本文書は `feature/refactor-clarify-processor-validation-boundary` での再整理結果を、規約として残すためのものです（`feature/add-internals-processor-conventions-doc` の commit `aa3c589a` で 0040 を close した際に、本ノートを 0046 完了時に生やす方針が確定しています）。

## 不変条件

> **5 構造体のフィールドは、`Self::new()` を経由した組立で eager に検証される。フィールドは `pub(crate)` のため crate 外から直接組み立てることはできない。**

検証対象は以下 5 種類です。

- `output_url` / `input_url` の非空
- `stream_name` 指定時の非空（RTMP 3 構造体）
- `stream_id` / `passphrase` 指定時の非空（SRT）
- `output_*_track_id` / `input_*_track_id` の少なくとも片方が必須
- `RtmpPublisherOptions::max_buffered_frame_count >= 1`（型 `NonZeroUsize` で静的保証）

## 対象外

以下は本ノートが扱う「構造体側の eager validation」の対象外です。

- TLS 有効時の `cert_path` / `key_path` ペア性 → `get_cert_and_key_paths()` の lazy validation
- `rtmps://` 時の `cert_path` 必須 → 同上（TLS 有効時のみ呼ばれる）
- `keyLength requires passphrase` → `SrtInboundEndpoint::endpoint_config()` の lazy validation
- `tsbpd_delay_ms <= u16::MAX` → `tsbpd_delay_duration_to_millis` の lazy validation
- URL 構文妥当性 → 各 `run()` 冒頭の `parse_rtmp_url` / `parse_srt_url` / `parse_rtsp_input_url`
- `Some(PathBuf::new())`（空 PathBuf）の検証 → 現状の `get_cert_and_key_paths()` も検証していない

これらは「構造体組立時には正しいかどうか判定できない」または「組み合わせ条件のため、`run()` の特定経路でしか発火しない」性質のため、`run()` または専用関数内で遅延検証します。
詳細は後述「lazy validation を温存する箇所」を参照してください。

## 検証項目マトリクス

| 不変条件 | 違反時の挙動 | 保証場所 |
| --- | --- | --- |
| `output_url` / `input_url` 非空 | `Err::Empty*Url` | 各 `new()` |
| `stream_name` (指定時) 非空 | `Err::EmptyStreamName` | RTMP 3 構造体の `new()` |
| `stream_id` (指定時) 非空 | `Err::EmptyStreamId` | `SrtInboundEndpoint::new()` |
| `passphrase` (指定時) 非空 | `Err::EmptyPassphrase` | `SrtInboundEndpoint::new()` |
| 少なくとも片方の track_id 必須 | `Err::NoTrackId` | 各 `new()` |
| `max_buffered_frame_count >= 1` | 型 `NonZeroUsize` で静的保証 | `RtmpPublisherOptions` 型 |
| `cert_path` / `key_path` のペア性 (TLS 時) | `get_cert_and_key_paths()` が Err | 既存 (`src/rtmp/{inbound,outbound}_endpoint.rs`) |
| `keyLength requires passphrase` | `endpoint_config()` が Err | 既存 (`src/srt/inbound_endpoint.rs`) |
| `tsbpd_delay_ms <= u16::MAX` | `tsbpd_delay_duration_to_millis` が Err | 既存 (`src/srt/inbound_endpoint.rs`) |
| URL 構文妥当性 | `parse_*_url` が Err | 既存 (各 `run()` 冒頭) |

## 5 構造体ごとの責務分担

| 構造体 | コンストラクタ | エラー型 | コンストラクタが弾く不変条件 |
| --- | --- | --- | --- |
| `RtmpInboundEndpoint` | `RtmpInboundEndpoint::new()` | `RtmpInboundEndpointBuildError` | `EmptyInputUrl` / `EmptyStreamName` / `NoTrackId` |
| `RtmpOutboundEndpoint` | `RtmpOutboundEndpoint::new()` | `RtmpOutboundEndpointBuildError` | `EmptyOutputUrl` / `EmptyStreamName` / `NoTrackId` |
| `RtmpPublisher` | `RtmpPublisher::new()` | `RtmpPublisherBuildError` | `EmptyOutputUrl` / `EmptyStreamName` / `NoTrackId` |
| `SrtInboundEndpoint` | `SrtInboundEndpoint::new()` | `SrtInboundEndpointBuildError` | `EmptyInputUrl` / `EmptyStreamId` / `EmptyPassphrase` / `NoTrackId` |
| `RtspSubscriber` | `RtspSubscriber::new()` | `RtspSubscriberBuildError` | `EmptyInputUrl` / `NoTrackId` |

各 `*BuildError` は `#[derive(Debug)]` と `impl std::fmt::Display` のみを実装します。
`#[derive(Clone)]` および `impl std::error::Error` は実装しません（`Err` 値は `?` 経由で即時上流伝播するため保持されず、`From<*BuildError> for crate::Error` 自動変換も採用していないため）。

## コンストラクタ強制 + `NonZeroUsize` ハイブリッドの設計判断

「コンストラクタ強制（フィールドを `pub(crate)` にして `new()` を必須経路にする）」と「型レベル保証（`NonZeroUsize`）」を組み合わせています。

- フィールド可視性を `pub(crate)` に下げることで「crate 外からの構造体リテラル組立は構文的に不可能」になります。crate 内（obsws coordinator）からの fields read は引き続き可能です。
- `max_buffered_frame_count` だけは `NonZeroUsize` で型レベルに `>= 1` を保証する経路に乗せます。`tokio::sync::mpsc::channel(0)` の panic 経路を構造的に排除するためです。
- `#[non_exhaustive]` は `shiguredo-rust` 規約で原則禁止のため使いません。crate 外利用は無い前提で、フィールド可視性のみで「`new()` 経由必須」を達成します。

## lazy validation を温存する箇所

以下は `run()` または専用関数内で遅延検証します（コンストラクタには含めません）。

- `cert_path` / `key_path` のペア性: TLS 有効時の `get_cert_and_key_paths()` で Err を返す。`rtmp://`（TLS 無効）時は呼ばれないため、`Some(PathBuf::new())` のような不完全な指定が残っていても無害です。
- `keyLength requires passphrase`: 組み合わせ条件のため、`endpoint_config()` 内で Err を返します。`KeyLength` 単独で `Some` でも `passphrase` が `Some` ならば正常系です。
- `tsbpd_delay_ms <= u16::MAX`: `tsbpd_delay_duration_to_millis` で `u16::try_from` した結果を返します。
- URL 構文妥当性: 各 `run()` 冒頭の `parse_*_url` で Err を返します。文字列としての非空はコンストラクタで保証していますが、`rtmp://` スキーム等の構文要件は遅延検証です。

これらをコンストラクタに移そうとすると、TLS 有効/無効・組み合わせ条件・外部クレート依存（URL parser）が混入するため、組立時点では「文字列が空でないか」だけを高速に判定し、より重い検証は `run()` に委ねる二段構えとしました。

## obsws 経路の責務

obsws 経路（`src/obsws/coordinator/` および `src/obsws/source/`）では、5 構造体を組み立てる際に必ず `new()?` を経由します。
受け取った `*BuildError` は以下のメッセージプレフィックスで文字列化し、`BuildObswsRecordSourcePlanError::InvalidInput(String)` または `crate::Error::new(format!("..."))` に変換します。

```
invalid <module_snake_case> config: {e}
```

`<module_snake_case>` は `rtmp_inbound` / `srt_inbound` / `rtsp_subscriber` / `rtmp_outbound_endpoint` / `rtmp_publisher` のいずれかです。

`is_source_startable` 関数（obsws の source 用 startable 判定）は `input_url.is_some()` のみを見ており、`Some("")` のような空文字は弾きません。
本ノートの責務分担では、空文字を弾くのは `new()?` 経由の組立時点に集約します。
obsws WebSocket 経由で空文字が来た場合、startable 判定では `true` を返しますが、後段の `build_record_source_plan` 内で `new()?` が Err となり、`BuildObswsRecordSourcePlanError::InvalidInput` 経由でクライアントに返ります。

## 新規 processor 構造体を追加する際のチェックリスト

新規に「外部入力経路（RTMP/SRT/RTSP のような）または外部出力経路の processor 構造体」を追加するときは、以下を満たすこと。

- 構造体本体のフィールドは `pub(crate)`（struct 自体は `pub` 可）
- `pub fn new(...) -> Result<Self, *BuildError>` のコンストラクタを設ける
- `*BuildError` は `#[derive(Debug)]` + `impl std::fmt::Display`。`Clone` 派生と `std::error::Error` 実装はしない
- バッファサイズ等の `>= 1` を必須とする整数フィールドは `NonZeroUsize` 等で型レベルに保証する
- 検証可能な不変条件は `new()` に集約。組み合わせ条件・TLS の有無で発火する検証・URL 構文等は `run()` または専用関数で遅延検証する
- 検証項目を本ノートの「検証項目マトリクス」と「5 構造体ごとの責務分担」表に追記する
- obsws 経路から組み立てる場合は `new()?` 呼びに揃え、エラーメッセージプレフィックスを `"invalid <module_snake_case> config: {e}"` で統一する
- `tests/<dir>_<module>_tests.rs` を新設して、正常系および各 `*BuildError` バリアントを `assert!(matches!(...))` で覆う

## 関連

- `src/rtmp/inbound_endpoint.rs` / `src/rtmp/outbound_endpoint.rs` / `src/rtmp/publisher.rs` / `src/srt/inbound_endpoint.rs` / `src/rtsp/subscriber.rs`
- `src/obsws/coordinator/` および `src/obsws/source/`（obsws 経路からの組立点）
- `src/error.rs`（`crate::Error` の設計方針：`From<E> for Error` 自動変換は実装しない）
- [`sample_entry_invariant.md`](sample_entry_invariant.md)（writer 入口の sample_entry 不変条件）
- `feature/refactor-remove-unused-processor-json-impls`（merge commit `42979dae`）: 旧 `TryFrom` 経路の削除
- `feature/refactor-clarify-processor-validation-boundary`: 本ノートの成果物となるブランチ
- `feature/add-internals-processor-conventions-doc`（commit `aa3c589a`）: 本ノートを生やす方針を決めた close 判断
