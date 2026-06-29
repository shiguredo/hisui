# 5 processor 構造体の validation 責務分担を確定して暴露面を塞ぐ

- Priority: Low
- Created: 2026-06-18
- Completed: 2026-06-29
- Model: Claude Opus 4.7
- Reporter: @sile
- Branch: feature/refactor-clarify-processor-validation-boundary
- Polished: 2026-06-26

## 目的

closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`) で次の 5 構造体の `impl TryFrom<nojson::RawJsonValue>` と `impl DisplayJson` が削除され、そこに集約されていた不変条件 (validation) が同時にコード上から消えた。

対象 5 構造体:

- `RtmpInboundEndpoint` (`src/rtmp/inbound_endpoint.rs:18-24`)
- `RtmpOutboundEndpoint` (`src/rtmp/outbound_endpoint.rs:33-40`)
- `RtmpPublisher` (`src/rtmp/publisher.rs:31-38`)
- `SrtInboundEndpoint` (`src/srt/inbound_endpoint.rs:21-33`)
- `RtspSubscriber` (`src/rtsp/subscriber.rs:32-37`)

5 構造体はいずれも `pub struct` + `pub` フィールド公開のままで、現在は obsws 経路の 5 箇所のみが組立に使う。`RtmpPublisherOptions::default()` が `max_buffered_frame_count: 1000` を返すため即時 panic は起きていないが、`pub` フィールドへの将来の代入で型システムから保証されない暴露面が残る。本 issue でこれを塞ぐ。

不変条件の所在は「採用方針」節の「検証項目マトリクス」に集約する (目的節での個別列挙は省略)。

## 優先度根拠

Low。

- 現状の呼び出し元 (obsws coordinator + obsws source 5 箇所) では `Default::default()` 経由で安全な値が入っており、即時の事故は起きていない
- 将来別経路を増やすときの落とし穴 + 設計判断の明文化のため、利用者影響ゼロのうちに設計を確定する価値あり

closed issue 0040 (`feature/add-internals-processor-conventions-doc` / commit `aa3c589a` / 2026-06-19 close) の close 判定で「validation 責務分担ノートは本 issue 完了時に `docs/internals/` 配下に生まれる」と明記されており、本 issue 完了時に対応ドキュメントを生む責務を負う。

## 現状

### 5 構造体の派生属性

| 構造体 | 派生属性 |
| --- | --- |
| `RtmpPublisher` (`src/rtmp/publisher.rs:31`) | `#[derive(Debug, Clone)]` |
| `RtmpInboundEndpoint` (`src/rtmp/inbound_endpoint.rs:18`) | **派生なし** |
| `RtmpOutboundEndpoint` (`src/rtmp/outbound_endpoint.rs:33`) | `#[derive(Debug, Clone)]` |
| `SrtInboundEndpoint` (`src/srt/inbound_endpoint.rs:21`) | **派生なし** |
| `RtspSubscriber` (`src/rtsp/subscriber.rs:32`) | `#[derive(Debug, Clone)]` |

本 issue では派生は **現状維持** (派生なし 2 構造体に追加しない)。派生統一は別 issue 候補とし「スコープ外」節で明示。

### Options 構造体の現状

- `RtmpPublisherOptions` (`src/rtmp/publisher.rs:11-21`): `pub max_buffered_frame_count: usize`、`#[derive(Debug, Clone)]` + 手書き `Default`
- `RtmpInboundEndpointOptions` (`src/rtmp/inbound_endpoint.rs:8-15`): `pub cert_path: Option<PathBuf>` / `pub key_path: Option<PathBuf>`、`#[derive(Debug, Clone, Default)]`
- `RtmpOutboundEndpointOptions` (`src/rtmp/outbound_endpoint.rs:24-31`): 同上構成、`#[derive(Debug, Clone, Default)]`
- SRT には Options 構造体がなく、`stream_id` / `passphrase` / `key_length` / `tsbpd_delay_ms` が `SrtInboundEndpoint` 本体に直接生えている

### obsws state パーサの現状

`src/obsws/state/types.rs` の `parse_optional_string_setting` (定義: line 720) は値の取り出しのみで、空文字検証は行わない。本 issue ではコンストラクタ側で空文字を `Err` で弾く方針とし、`parse_optional_string_setting` には手を入れない (色文字 / mp4 path 等、他の利用箇所への影響を避けるため)。

### tokio mpsc panic 仕様

`tokio::sync::mpsc::channel(buffer)` は `buffer == 0` で `assert!(buffer > 0)` panic 確定。`RtmpPublisher` のみが呼ぶ (`src/rtmp/publisher.rs:82`)。本 issue で `NonZeroUsize` 化により型レベルに昇格。

### 5 構造体の `run()` における track_id 両方 None の挙動差異 (参考)

`TryFrom` 削除前は eager に弾かれていたが、現状は構造体ごとに挙動が異なる。コンストラクタが `NoTrackId` を Err で弾けば下記差異は表面化しない。

- `src/rtmp/publisher.rs:135` / `src/rtmp/outbound_endpoint.rs:136`: `(None, None) => break,` で即終了
- `src/rtmp/inbound_endpoint.rs:149-`: TCP listener が回り続け、空のまま接続を受け付ける
- `src/srt/inbound_endpoint.rs:241-`: UDP socket と SRT セッションが立ち上がり続ける
- `src/rtsp/subscriber.rs:92-`: RTSP セッションを張りに行く

### closed 0041 で削除された helper との対応 (網羅性確認)

| 削除 helper | 旧責務 | 本 issue でのカバー |
| --- | --- | --- |
| `parse_optional_non_empty_string` (SRT) | `stream_id` / `passphrase` 空文字を None 化 | コンストラクタが空文字を `Err` で弾く (None 化はしない) |
| `parse_optional_key_length` (SRT) | JSON `u32` → `KeyLength` enum 変換 | JSON 経路廃止のため不要。現状 `Option<KeyLength>` で型安全 |
| `key_length_to_rpc_value` (SRT) | `KeyLength` → JSON `u32` 変換 | JSON 経路廃止のため不要 |
| `validate_input_url` (RTSP) | RTSP URL 構文検証 | `run()` 冒頭の `parse_rtsp_input_url` で等価検証 |

### crate 外利用の確認

本 crate は `Cargo.toml` に `[lib]` セクションを持たないが、`src/lib.rs` が自動検出され library 化されている。以下の grep で確認した:

- `grep -rn "hisui::rtmp::\|hisui::srt::\|hisui::rtsp::" examples/ pbt/ fuzz/`
- `grep -rn "RtmpPublisherOptions\s*{\|RtmpInboundEndpointOptions\s*{\|RtmpOutboundEndpointOptions\s*{" examples/ pbt/ fuzz/ src/`

結果は対象 5 構造体および `*Options` 構造体のリテラル組立はゼロ (`src/obsws/coordinator/{output_rtmp,output_stream}.rs` の `options: Default::default()` 経由参照と `src/rtmp/publisher.rs` 内の `Default` 実装のみ)。`pub(crate)` 化と `NonZeroUsize` 化による既存呼出元影響は無い。

## 採用方針

**コンストラクタ強制 + `NonZeroUsize` + Options 集約 + 専用エラー型** のハイブリッドを採用する。`#[non_exhaustive]` は `shiguredo-rust` 規約で原則禁止のため使わない。

### 設計判断記録

- **ブランチ prefix `feature/refactor-`**: `pub` → `pub(crate)` フィールド化と `usize` → `NonZeroUsize` 型変更は形式上 API 表面変更だが、crate 外利用ゼロ (「現状」節「crate 外利用の確認」で実証) のため `change-` ではなく実態に合った `refactor-` を採用
- **`*BuildError` への `Display` 実装**: `crate::Error` は意図的に `Display` 不実装 + `From<E> for Error` 系自動変換を避ける設計 (`src/error.rs:74-79` の `[NOTE]` コメントで「`Error` を `Error` のまま再ラップした場合に reason / location / backtrace が重複してエラー出力が冗長になる」ことを理由として明記) だが、本 issue の `*BuildError` は obsws 経路で `format!("{e}")` 経由で `BuildObswsRecordSourcePlanError::InvalidInput(String)` または `crate::Error::new(format!(...))` に変換するために `Display` 必須。本 issue は `From<*BuildError> for crate::Error` 自動変換を採用しない方針 (採用要素 4 末尾) のため `crate::Error` の設計思想とは衝突しない
- **テストファイル命名 `tests/<dir>_<module>_tests.rs` 形式**: `shiguredo-rust` 規約「`tests/test_<module>.rs`」はディレクトリモジュール (`src/<dir>/<module>.rs`) への対応規則が SKILL.md に未定義。既存 `tests/` 配下 10 ファイル (`e2e.rs` 含む) の実態は `*_tests.rs` パターン (`reader_webm_tests.rs` ↔ `src/webm/reader.rs` 等、6 ファイル) が多数派、`test_*.rs` は少数派 (3 ファイル)、`e2e.rs` 形式が 1 ファイル。本 issue は多数派 + ディレクトリモジュール対応先例 `reader_webm_tests.rs` に揃え `tests/rtmp_publisher_tests.rs` 等の `<dir>_<module>_tests.rs` 命名を採用する

### 採用要素 1: 本体 5 構造体のフィールド可視性を `pub(crate)` に格下げ、`pub fn new() -> Result<Self, _>` を必須経路に

- **本体 5 構造体の struct 自体は `pub` のまま維持** し、**フィールドのみ `pub(crate)` 化** する。これにより `pub enum ObswsSourceRequest` の variant 内で 5 構造体を payload として保持しても `pub` 型を `pub` enum に保持するだけなので可視性 lint 警告は出ず、`pub fn create_processor` も `pub` のまま維持できる
- 同 crate 内 (obsws 配下) からのフィールド read アクセスは `pub(crate)` で継続可能 (`subscriber.input_url.clone()` 等)
- 既存 lazy validation (`endpoint_config()` / `tsbpd_delay_duration_to_millis()` / `get_cert_and_key_paths()`) は可視性も含めて現状の `fn` のまま維持

### 採用要素 2: Options 構造体のフィールドは `pub` のまま維持

- 4 つの `*Options` 構造体 (既存 3 つ + 新設 `SrtInboundEndpointOptions`) のフィールドは **すべて `pub` のまま維持** する。理由:
  - integration test (`tests/*_tests.rs`) から `RtmpPublisherOptions::default().max_buffered_frame_count` 等のフィールド read で退行検知する必要がある
  - integration test で `SrtInboundEndpointOptions { stream_id: Some("".to_owned()), ... }` のような空文字ケースを構築する必要がある
  - Options 自体は validation の対象ではなく、本体 5 構造体の `new()` で validation を実施する。Options が `pub` フィールドでも、本体構造体が `pub(crate)` フィールド + `new()` 強制なら、不正な Options を渡しても `new()` で `Err` を返すため暴露面は塞がる
- `RtmpPublisherOptions::max_buffered_frame_count` も `pub` のまま維持。型を `NonZeroUsize` に昇格することで型レベル保証で暴露面ゼロ

### 採用要素 3: `RtmpPublisherOptions::max_buffered_frame_count` を `usize` → `NonZeroUsize` に昇格

- `tokio::sync::mpsc::channel(0)` panic を型レベルで排除
- 既存パターン: `src/encoder.rs:62-` / `src/subcommand_server.rs:119-` / `src/obsws/coordinator/output.rs:723, 733` で `NonZeroUsize` 使用済み
- `RtmpPublisherOptions::default()` は `NonZeroUsize::new(1000).expect("non-zero constant")` で初期化 (既存慣用 `src/obsws/coordinator/output.rs:723` に揃える)
- 現状 `RtmpPublisherOptions { max_buffered_frame_count: usize }` リテラル組立は `src/rtmp/publisher.rs:23-29` の `Default` 実装内のみ (「現状」節「crate 外利用の確認」で実証済み)。ここを書き換えるだけで完結
- `mpsc::channel(self.options.max_buffered_frame_count)` (line 82) は `.get()` 経由 `mpsc::channel(self.options.max_buffered_frame_count.get())` に書き換える
- `RtmpPublisherOptions::max_buffered_frame_count` の docstring 末尾に「`NonZeroUsize` で `>= 1` を型レベル保証」を追記

### 採用要素 4: 各構造体に専用 `*BuildError` enum + `Display` 実装

- 各 `new()` の戻り型 `Result<Self, *BuildError>` の `Err` バリアントは enum で表現 (フィールド無しの identifier のみ)
- 各 `*BuildError` に `impl std::fmt::Display` を **必須** で実装 (obsws 経路で `format!("{e}")` を利用するため)
- 派生は `#[derive(Debug)]` のみ。`Clone` は派生 **してはならない** (Err 値は `?` 経由で即時上流伝播するため保持されない)
- `impl std::error::Error` は **実装しない** (`From<*BuildError> for crate::Error` 自動変換は採用しないため不要)
- `*BuildError` を `src/lib.rs` で `pub use` 再エクスポートしない (`shiguredo-rust` 規約「re-export は基本的にやらないこと」と整合)

### 採用要素 5: SRT も他 4 構造体と同じ「本体 + Options」スタイルに揃える

- `SrtInboundEndpoint::new` の引数が 7 個になり `#[expect(clippy::too_many_arguments)]` が必要になるため、`stream_id` / `passphrase` / `key_length` / `tsbpd_delay_ms` を `SrtInboundEndpointOptions` に集約する
- `SrtInboundEndpointOptions` 派生は `#[derive(Debug, Clone, Default)]` (既存他 2 Options と整合)
- `SrtInboundEndpointOptions` のフィールド可視性は `pub` (採用要素 2 の方針)、struct 自体も `pub`

### `*BuildError` の Display メッセージ一覧

| Err バリアント | Display メッセージ |
| --- | --- |
| `RtmpPublisherBuildError::EmptyOutputUrl` | `"output_url must not be empty"` |
| `RtmpPublisherBuildError::EmptyStreamName` | `"stream_name must not be empty when specified"` |
| `RtmpPublisherBuildError::NoTrackId` | `"at least one of input_audio_track_id / input_video_track_id must be set"` |
| `RtmpInboundEndpointBuildError::EmptyInputUrl` | `"input_url must not be empty"` |
| `RtmpInboundEndpointBuildError::EmptyStreamName` | `"stream_name must not be empty when specified"` |
| `RtmpInboundEndpointBuildError::NoTrackId` | `"at least one of output_audio_track_id / output_video_track_id must be set"` |
| `RtmpOutboundEndpointBuildError::EmptyOutputUrl` | `"output_url must not be empty"` |
| `RtmpOutboundEndpointBuildError::EmptyStreamName` | `"stream_name must not be empty when specified"` |
| `RtmpOutboundEndpointBuildError::NoTrackId` | `"at least one of input_audio_track_id / input_video_track_id must be set"` |
| `SrtInboundEndpointBuildError::EmptyInputUrl` | `"input_url must not be empty"` |
| `SrtInboundEndpointBuildError::EmptyStreamId` | `"stream_id must not be empty when specified"` |
| `SrtInboundEndpointBuildError::EmptyPassphrase` | `"passphrase must not be empty when specified"` |
| `SrtInboundEndpointBuildError::NoTrackId` | `"at least one of output_audio_track_id / output_video_track_id must be set"` |
| `RtspSubscriberBuildError::EmptyInputUrl` | `"input_url must not be empty"` |
| `RtspSubscriberBuildError::NoTrackId` | `"at least one of output_audio_track_id / output_video_track_id must be set"` |

### 各構造体のコンストラクタ シグネチャ

引数順序は **`audio` → `video`** で 5 構造体すべて統一する (`RtspSubscriber` の現状フィールド宣言順 `video, audio` から並べ替える)。`RtspSubscriber` のフィールド宣言順も `output_audio_track_id, output_video_track_id` に揃える。

`Display` 実装は採用要素 4 のメッセージ一覧に従い、5 つの `*BuildError` に対して同形式で実装する (例示は `RtmpPublisherBuildError` のみ)。

#### `RtmpPublisher::new`

```rust
#[derive(Debug)]
pub enum RtmpPublisherBuildError {
    EmptyOutputUrl,
    EmptyStreamName,
    NoTrackId,
}

impl std::fmt::Display for RtmpPublisherBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyOutputUrl => write!(f, "output_url must not be empty"),
            Self::EmptyStreamName => write!(f, "stream_name must not be empty when specified"),
            Self::NoTrackId => write!(f, "at least one of input_audio_track_id / input_video_track_id must be set"),
        }
    }
}

impl RtmpPublisher {
    pub fn new(
        output_url: String,
        stream_name: Option<String>,
        input_audio_track_id: Option<TrackId>,
        input_video_track_id: Option<TrackId>,
        options: RtmpPublisherOptions,
    ) -> Result<Self, RtmpPublisherBuildError> { ... }
}
```

#### `RtmpInboundEndpoint::new`

```rust
#[derive(Debug)]
pub enum RtmpInboundEndpointBuildError {
    EmptyInputUrl,
    EmptyStreamName,
    NoTrackId,
}

impl RtmpInboundEndpoint {
    pub fn new(
        input_url: String,
        stream_name: Option<String>,
        output_audio_track_id: Option<TrackId>,
        output_video_track_id: Option<TrackId>,
        options: RtmpInboundEndpointOptions,
    ) -> Result<Self, RtmpInboundEndpointBuildError> { ... }
}
```

`cert_path` / `key_path` のペア性 / `rtmps://` 時必須は `new()` に含めない (`get_cert_and_key_paths()` の lazy validation を維持)。

#### `RtmpOutboundEndpoint::new`

```rust
#[derive(Debug)]
pub enum RtmpOutboundEndpointBuildError {
    EmptyOutputUrl,
    EmptyStreamName,
    NoTrackId,
}

impl RtmpOutboundEndpoint {
    pub fn new(
        output_url: String,
        stream_name: Option<String>,
        input_audio_track_id: Option<TrackId>,
        input_video_track_id: Option<TrackId>,
        options: RtmpOutboundEndpointOptions,
    ) -> Result<Self, RtmpOutboundEndpointBuildError> { ... }
}
```

#### `SrtInboundEndpoint::new`

```rust
#[derive(Debug, Clone, Default)]
pub struct SrtInboundEndpointOptions {
    pub stream_id: Option<String>,
    pub passphrase: Option<String>,
    pub key_length: Option<KeyLength>,
    pub tsbpd_delay_ms: Option<Duration>,
}

#[derive(Debug)]
pub enum SrtInboundEndpointBuildError {
    EmptyInputUrl,
    EmptyStreamId,
    EmptyPassphrase,
    NoTrackId,
}

impl SrtInboundEndpoint {
    pub fn new(
        input_url: String,
        output_audio_track_id: Option<TrackId>,
        output_video_track_id: Option<TrackId>,
        options: SrtInboundEndpointOptions,
    ) -> Result<Self, SrtInboundEndpointBuildError> { ... }
}
```

`keyLength requires passphrase` / `tsbpd_delay_ms <= u16::MAX` は `new()` に含めない (`endpoint_config()` / `tsbpd_delay_duration_to_millis` の lazy validation を維持)。`endpoint_config()` 内の `self.stream_id` / `self.passphrase` / `self.key_length` / `self.tsbpd_delay_ms` 参照は `self.options.*` に書き換える。

#### `RtspSubscriber::new`

```rust
#[derive(Debug)]
pub enum RtspSubscriberBuildError {
    EmptyInputUrl,
    NoTrackId,
}

impl RtspSubscriber {
    pub fn new(
        input_url: String,
        output_audio_track_id: Option<TrackId>,
        output_video_track_id: Option<TrackId>,
    ) -> Result<Self, RtspSubscriberBuildError> { ... }
}
```

### 検証項目マトリクス

| 不変条件 | 違反時の挙動 | 保証場所 |
| --- | --- | --- |
| `output_url` / `input_url` 非空 | `Err::Empty*Url` | 各 `new()` |
| `stream_name` (指定時) 非空 | `Err::EmptyStreamName` | RTMP 3 構造体の `new()` |
| `stream_id` (指定時) 非空 | `Err::EmptyStreamId` | `SrtInboundEndpoint::new()` |
| `passphrase` (指定時) 非空 | `Err::EmptyPassphrase` | `SrtInboundEndpoint::new()` |
| 少なくとも片方の track_id 必須 | `Err::NoTrackId` | 各 `new()` |
| `max_buffered_frame_count >= 1` | 型 `NonZeroUsize` で静的保証 | `RtmpPublisherOptions` 型 |
| `cert_path` / `key_path` のペア性 (TLS 時) | `get_cert_and_key_paths()` が Err | 既存 (`src/rtmp/{inbound,outbound}_endpoint.rs`) |
| `keyLength requires passphrase` | `endpoint_config()` が Err | 既存 (`src/srt/inbound_endpoint.rs:290`) |
| `tsbpd_delay_ms <= u16::MAX` | `tsbpd_delay_duration_to_millis` が Err | 既存 (`src/srt/inbound_endpoint.rs:358`) |
| URL 構文妥当性 | `parse_*_url` が Err | 既存 (各 `run()` 冒頭) |

### obsws 組立点の書き換え

obsws 経路 5 箇所のリテラル組立を `new()?` 呼びに書き換える。エラーメッセージプレフィックスは `"invalid <module_snake_case> config: {e}"` で統一する (`<module_snake_case>` は `rtmp_inbound` / `srt_inbound` / `rtsp_subscriber` / `rtmp_outbound_endpoint` / `rtmp_publisher`)。

```rust
// src/obsws/source/rtmp_inbound.rs:26-32
let endpoint = crate::rtmp::inbound_endpoint::RtmpInboundEndpoint::new(
    input_url.to_owned(),
    settings.stream_name.clone(),
    Some(raw_audio_track_id.clone()),
    Some(raw_video_track_id.clone()),
    Default::default(),
)
.map_err(|e| BuildObswsRecordSourcePlanError::InvalidInput(format!("invalid rtmp_inbound config: {e}")))?;

// src/obsws/source/srt_inbound.rs:26-34
let endpoint = crate::srt::inbound_endpoint::SrtInboundEndpoint::new(
    input_url.to_owned(),
    Some(raw_audio_track_id.clone()),
    Some(raw_video_track_id.clone()),
    crate::srt::inbound_endpoint::SrtInboundEndpointOptions {
        stream_id: settings.stream_id.clone(),
        passphrase: settings.passphrase.clone(),
        key_length: None,
        tsbpd_delay_ms: None,
    },
)
.map_err(|e| BuildObswsRecordSourcePlanError::InvalidInput(format!("invalid srt_inbound config: {e}")))?;

// src/obsws/source/rtsp_subscriber.rs:26-30
let subscriber = crate::rtsp::subscriber::RtspSubscriber::new(
    input_url.to_owned(),
    Some(raw_audio_track_id.clone()),
    Some(raw_video_track_id.clone()),
)
.map_err(|e| BuildObswsRecordSourcePlanError::InvalidInput(format!("invalid rtsp_subscriber config: {e}")))?;

// src/obsws/coordinator/output_rtmp.rs:282-288 (戻り型 crate::Result<()>)
let endpoint = crate::rtmp::outbound_endpoint::RtmpOutboundEndpoint::new(
    output_url.to_owned(),
    stream_name.map(|s| s.to_owned()),
    Some(run.audio.encoded_track_id.clone()),
    Some(run.video.encoded_track_id.clone()),
    Default::default(),
)
.map_err(|e| crate::Error::new(format!("invalid rtmp_outbound_endpoint config: {e}")))?;

// src/obsws/coordinator/output_stream.rs:383-389
let publisher = crate::rtmp::publisher::RtmpPublisher::new(
    output_url.to_owned(),
    stream_key.map(|s| s.to_owned()),
    Some(run.audio.encoded_track_id.clone()),
    Some(run.video.encoded_track_id.clone()),
    Default::default(),
)
.map_err(|e| crate::Error::new(format!("invalid rtmp_publisher config: {e}")))?;
```

`From<*BuildError> for crate::Error` 自動変換は採用しない (`src/error.rs:75-82` の方針と整合)。

### docs/internals/ ノート

新規 `docs/internals/processor_validation.md` を作成する。既存 `docs/internals/sample_entry_invariant.md` の節構成 (概要 / 不変条件 / 対象外 / 適用範囲 / 確立できない場合の扱い / writer 側の前提) を参考にしつつ、本 issue 固有の事情 (5 構造体の責務分担 + 新規追加チェックリスト) に応じて節を加えた以下の 9 節構成とする。`shiguredo-issues` 規約「ソースコードに issue 番号を持ち込まない」は docs/ も対象 (規約 line 43) のため、本ノート内では issue 番号や issue パス参照ではなく **ブランチ名 + merge commit hash** で過去経緯を参照する。

#### 章構成

1. 概要 (validation の所在を明文化する目的)
2. 不変条件 (5 構造体共通の不変条件を引用ブロックで明示)
3. 対象外 (本ノートが扱わない検証項目)
4. 検証項目マトリクス (採用方針節の表をそのまま転記)
5. 5 構造体ごとの責務分担表
6. lazy validation を温存する箇所 (`get_cert_and_key_paths` / `endpoint_config` / `tsbpd_delay_duration_to_millis` / `parse_*_url`) と再実装しない理由
7. obsws 経路の責務 (`is_source_startable` は空文字を弾かず、`new()?` で弾く設計)
8. 新規 processor 構造体を追加する際のチェックリスト
9. 関連 (`feature/refactor-remove-unused-processor-json-impls` (merge `42979dae`) / `feature/add-internals-processor-conventions-doc` (commit `aa3c589a`) / 5 構造体ソースパス / `sample_entry_invariant.md`)

## 実装順序

各ステップ完了後に `cargo check --workspace && cargo test --workspace` を通す (中間 commit も同様)。`shiguredo-git` 規約「1 コミット = 1 論理変更」に従い、原則 1 ステップ = 1 commit。

1. **RTMP 3 構造体と RTSP に `*BuildError` enum (`Debug` + `Display`) と `pub fn new()` を追加**。`pub` フィールドは残したまま、obsws 経路もまだ書き換えない。`RtspSubscriber` のフィールド宣言順を `audio, video` に並べ替える
2. **SRT 集約**を 1 commit で実施: `SrtInboundEndpointOptions` 新設 (フィールド `pub`、`#[derive(Debug, Clone, Default)]`) + `SrtInboundEndpoint` フィールド構造を `options` 集約に変更 + `endpoint_config()` 内の `self.*` 参照を `self.options.*` に書き換え + `SrtInboundEndpointBuildError` + `SrtInboundEndpoint::new()` 追加 + `src/obsws/source/srt_inbound.rs:26-34` の本体リテラル組立を `SrtInboundEndpoint::new(...)?` 呼びに置換 (本ステップ内で SRT 経路全体の整合性を完成させる。中途半端な状態を残さない) + `src/obsws/source/srt_inbound.rs:81-82, 102-103` の `mod tests` assertion を `endpoint.options.stream_id` / `endpoint.options.passphrase` に書き換え
3. **残り 4 つの obsws 経路をリテラルから `new()?` 呼びに書き換え** (rtmp_inbound / rtsp_subscriber / output_rtmp / output_stream)
4. **本体 5 構造体のフィールド可視性を `pub` → `pub(crate)` に格下げ** (struct 自体は `pub` 維持。Options 構造体は `pub` のまま、フィールドも `pub` 維持)
5. **`RtmpPublisherOptions::max_buffered_frame_count` を `usize` → `NonZeroUsize` に変更** + `Default` 修正 + `src/rtmp/publisher.rs:82` の `mpsc::channel(...)` を `.get()` 経由に修正 + docstring 補強
6. **doc コメントを追加** (詳細は次節)
7. **`tests/<dir>_<module>_tests.rs` 5 ファイルを追加**
8. **`docs/internals/processor_validation.md` を新規作成**

### doc コメント追加

5 構造体すべてに以下 3 箇所の doc コメントを追加する (CLAUDE.md「コメントは全て日本語」規約に従い日本語)。`*Options` 構造体の既存 docstring は維持 (新規追加なし、ただし `RtmpPublisherOptions::max_buffered_frame_count` のみ `NonZeroUsize` 化の旨を末尾に追記)。SRT 新設 `SrtInboundEndpointOptions` には「`SrtInboundEndpoint` 用オプション (`stream_id` / `passphrase` / `key_length` / `tsbpd_delay_ms`)」程度の 1 行 doc を追加。

参考実例 (`SrtInboundEndpoint` の場合):

```rust
/// SRT Inbound Endpoint
///
/// フィールドの不変条件は `Self::new()` で eager 検証される。
/// フィールドは `pub(crate)` のため crate 外からは `new()` 経由でのみ組み立てられる。
///
/// 以下の検証は遅延 (`run()` 内):
/// - URL 構文妥当性 (`parse_srt_url`)
/// - `keyLength requires passphrase` (`endpoint_config()`)
/// - `tsbpd_delay_ms <= u16::MAX` (`tsbpd_delay_duration_to_millis`)
pub struct SrtInboundEndpoint { ... }

/// `SrtInboundEndpoint` を構築する。以下を eager 検証する:
/// - `EmptyInputUrl`: `input_url` 非空
/// - `EmptyStreamId`: `options.stream_id` 指定時の非空
/// - `EmptyPassphrase`: `options.passphrase` 指定時の非空
/// - `NoTrackId`: `output_audio_track_id` / `output_video_track_id` の少なくとも一方が必須
pub fn new(...) -> Result<Self, SrtInboundEndpointBuildError> { ... }

/// `SrtInboundEndpoint::new()` が返す検証エラー。
pub enum SrtInboundEndpointBuildError { ... }
```

他 4 構造体も同じパターンで翻訳して書く。

## テスト戦略

5 構造体の各コンストラクタの正常系・異常系を integration test で検証する。本テスト群は新設 `new()` API のためのものであり、既存 `run()` 経路の回帰テストではない。PBT 化は不要 (列挙可能なエラー variant のみで property の追加カバレッジが薄い)。

### テストファイルと最小ケース

既存 `tests/` 10 ファイルのうち多数派の `*_tests.rs` パターン (6 ファイル) + ディレクトリモジュール対応先例 `reader_webm_tests.rs` ↔ `src/webm/reader.rs` に従い、`tests/<dir>_<module>_tests.rs` 形式で命名する。

- `tests/rtmp_publisher_tests.rs`
  - `new_accepts_with_stream_name_some`
  - `new_accepts_with_stream_name_none`
  - `new_accepts_with_audio_only_track_id`
  - `new_accepts_with_video_only_track_id`
  - `new_rejects_empty_output_url`
  - `new_rejects_empty_stream_name`
  - `new_rejects_both_track_ids_none`
  - `options_default_max_buffered_frame_count_is_1000` (`NonZeroUsize` 化後の退行検知)
- `tests/rtmp_inbound_endpoint_tests.rs` (7 基本ケース)
- `tests/rtmp_outbound_endpoint_tests.rs` (7 基本ケース)
- `tests/srt_inbound_endpoint_tests.rs` (9 ケース: 7 基本 + `new_rejects_empty_stream_id` + `new_rejects_empty_passphrase`)
- `tests/rtsp_subscriber_tests.rs` (5 ケース: 正常系 + audio only + video only + empty input_url + both track ids none)

`*BuildError` enum はフィールド無しなので `assert!(matches!(err, X::EmptyOutputUrl))` で十分 (`PartialEq` 派生は不要)。

### 参考スニペット (`tests/rtmp_publisher_tests.rs` の最初 3 ケース)

`hisui::TrackId` は `src/lib.rs:49-53` で `pub use media_pipeline::TrackId` 再エクスポート済み、`hisui::rtmp::publisher::*` は `pub mod rtmp` (`src/lib.rs:25`) 経由でアクセス可能。

```rust
use std::num::NonZeroUsize;
use hisui::{
    TrackId,
    rtmp::publisher::{RtmpPublisher, RtmpPublisherBuildError, RtmpPublisherOptions},
};

#[test]
fn new_accepts_with_stream_name_some() {
    // 正常系: stream_name 指定あり + audio + video track_id 両方指定
    let publisher = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpPublisherOptions::default(),
    )
    .expect("正常系のコンストラクタは成功する");
    let _ = publisher;
}

#[test]
fn new_rejects_empty_output_url() {
    // 空の output_url は EmptyOutputUrl で弾かれること
    let err = RtmpPublisher::new(
        "".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpPublisherOptions::default(),
    )
    .expect_err("空 output_url は弾く");
    assert!(matches!(err, RtmpPublisherBuildError::EmptyOutputUrl));
}

#[test]
fn options_default_max_buffered_frame_count_is_1000() {
    // デフォルト値が 1000 のままであること (NonZeroUsize 化後の退行検知)
    assert_eq!(
        RtmpPublisherOptions::default().max_buffered_frame_count,
        NonZeroUsize::new(1000).expect("non-zero constant"),
    );
}
```

`tests/srt_inbound_endpoint_tests.rs` での `empty_stream_id` / `empty_passphrase` テストは `SrtInboundEndpointOptions` のフィールドが `pub` のため integration test から直接組立可能:

```rust
let err = SrtInboundEndpoint::new(
    "srt://127.0.0.1:9000".to_owned(),
    Some(TrackId::new("audio")),
    Some(TrackId::new("video")),
    SrtInboundEndpointOptions {
        stream_id: Some("".to_owned()),
        passphrase: None,
        key_length: None,
        tsbpd_delay_ms: None,
    },
)
.expect_err("空 stream_id は弾く");
assert!(matches!(err, SrtInboundEndpointBuildError::EmptyStreamId));
```

obsws coordinator 側の既存 `mod tests` のフィールド read アクセスは:

- `src/obsws/source/srt_inbound.rs:81-82, 102-103` の `endpoint.stream_id` / `endpoint.passphrase` は実装ステップ 2 で `endpoint.options.*` に書き換え (構造変化追従)
- `src/obsws/source/{rtmp_inbound,rtsp_subscriber}.rs` の `endpoint.input_url` / `endpoint.stream_name` / `subscriber.input_url` 等は `pub(crate)` 化後も同 crate 内 read アクセスとして継続可能 (書き換え不要)

## 完了条件

- [ ] 5 構造体本体のフィールドが `pub(crate)` に格下げされ (struct 自体は `pub` 維持)、上記シグネチャの `pub fn new() -> Result<Self, *BuildError>` コンストラクタが追加されている
- [ ] 各 `*BuildError` enum が `#[derive(Debug)]` で派生 + `impl std::fmt::Display` を実装し、メッセージ文字列が上記「Display メッセージ一覧」表と完全一致する。`#[derive(Clone)]` も `impl std::error::Error` も追加していない
- [ ] 4 つの `*Options` (`RtmpPublisherOptions` / `RtmpInboundEndpointOptions` / `RtmpOutboundEndpointOptions` / 新設 `SrtInboundEndpointOptions`) のフィールドは `pub` のまま維持されている (テスト退行検知 + 空文字ケース構築のため)
- [ ] `RtmpPublisherOptions::max_buffered_frame_count` が `NonZeroUsize` に昇格し、`Default::default()` が `NonZeroUsize::new(1000).expect("non-zero constant")` で初期化されている
- [ ] `src/rtmp/publisher.rs:82` の `mpsc::channel(...)` が `.get()` 経由になっている
- [ ] `SrtInboundEndpointOptions` が新設され、フィールドは `pub`、struct も `pub`、派生 `#[derive(Debug, Clone, Default)]` で、`SrtInboundEndpoint` のフィールドが `input_url` / `output_audio_track_id` / `output_video_track_id` / `options: SrtInboundEndpointOptions` 構成になっている
- [ ] `SrtInboundEndpoint::endpoint_config()` 内の `self.stream_id` / `self.passphrase` / `self.key_length` / `self.tsbpd_delay_ms` 参照が `self.options.*` に書き換えられている
- [ ] obsws 経路の組立 5 箇所がすべて `new()?` 呼びに書き換えられ、エラーメッセージプレフィックスが `"invalid <module_snake_case> config: {e}"` で統一されている
- [ ] `src/obsws/source/srt_inbound.rs:81-82, 102-103` の `mod tests` assertion が `endpoint.options.stream_id` / `endpoint.options.passphrase` に書き換えられている
- [ ] `RtspSubscriber` のフィールド宣言順が `output_audio_track_id, output_video_track_id` に並べ替えられている
- [ ] 既存 `RtmpInboundEndpoint` / `SrtInboundEndpoint` の派生属性は **追加しない** (現状維持)。5 構造体本体に `Default` 実装を新規追加しない
- [ ] 各構造体に struct / `new()` / `*BuildError` の 3 箇所 doc コメントが追加されている (日本語、上記実例パターン)。`RtmpPublisherOptions::max_buffered_frame_count` の docstring に `NonZeroUsize` 化の旨を追記。新設 `SrtInboundEndpointOptions` に 1 行 doc コメントを追加
- [ ] `tests/<dir>_<module>_tests.rs` 5 ファイルが上記命名・最小ケースで追加されている
- [ ] 新規 `docs/internals/processor_validation.md` が上記章構成 (9 節) で作成されている
- [ ] 以下のコマンドがすべて通る (中間 commit でも同様):
  - `cargo fmt --all --check`
  - `cargo check --workspace`
  - `cargo check --workspace --no-default-features`
  - `cargo clippy --workspace --all-targets -- --deny warnings`
  - `cargo clippy --workspace --no-default-features -- --deny warnings`
  - `cargo test --workspace`
  - `cargo test --workspace --no-default-features` (5 構造体は feature gate に無関係なので影響無いが、`player` 等 default feature 無効化時のビルド整合性確認のため)

## スコープ外 (本 issue では扱わない、すべて別 issue 候補)

- `RtmpPublisher::run` 内の `parse_rtmp_url` 2 重呼び出し (`src/rtmp/publisher.rs:79, 89`)
- `RtmpInboundEndpointOptions` / `RtmpOutboundEndpointOptions` の `Some(PathBuf::new())` (空 PathBuf) の検証 (既存 `get_cert_and_key_paths()` も検証していない)
- 5 構造体の派生属性統一 (`RtmpInboundEndpoint` / `SrtInboundEndpoint` の派生なし → 他 3 構造体に揃えて `Debug + Clone` 派生)
- obsws `is_source_startable` の空文字弾き (本 issue の `new()?` で弾けば運用上問題ないと判断)

## CHANGES.md について

`CHANGES.md` には **追記しない**。

- 本 crate は実行ファイル `hisui` 単体として配布されており、`examples/` / `pbt/` / `fuzz/` 配下の crate 外利用も無い (「現状」節「crate 外利用の確認」参照)。`pub` → `pub(crate)` フィールド化による crate 外利用者影響はゼロ
- obsws WebSocket API 経由で「空 `stream_name` が拒否される」等の振る舞い変化はあるが、空文字を受け入れていた経路は実装バグであり、unreleased 機能 (`## develop` 配下の obsws 機能群) の中間修正に該当する。`shiguredo-changelog` 規約「開発ブランチ内の中間状態の修正は記載しないこと」に従い独立 `[FIX]` エントリは作らない

## 関連

- closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae` / 2026-06-18 完了): 本 issue 対象の不変条件が削除された直接の契機
- closed issue 0040 (`feature/add-internals-processor-conventions-doc` / commit `aa3c589a` / 2026-06-19 close): 本 issue の `docs/internals/processor_validation.md` 作成を 0040 の代替成果物として確定

## 解決方法 (2026-06-29)

`feature/refactor-clarify-processor-validation-boundary` で次を実装した。コード変更 13 コミット、16 ファイル + 整理 commit。

### 実装内容

- 5 構造体 (`RtmpInboundEndpoint` / `RtmpOutboundEndpoint` / `RtmpPublisher` / `SrtInboundEndpoint` / `RtspSubscriber`) に `*BuildError` enum (`Debug` 派生 + `Display` 実装) と `pub fn new() -> Result<Self, *BuildError>` を追加した
- 本体 5 構造体のフィールドを `pub` → `pub(crate)` に格下げし、`new()` を唯一の組立経路とした (struct 自体は `pub` 維持)
- `RtmpPublisherOptions::max_buffered_frame_count` を `usize` → `NonZeroUsize` に昇格し、`tokio::sync::mpsc::channel(0)` panic を型レベルで排除
- `SrtInboundEndpoint` のフィールドを `SrtInboundEndpointOptions` に集約し、他 4 構造体と「本体 + Options」スタイルを統一
- `RtspSubscriber` のフィールド宣言順 / `new()` 引数順を `audio, video` に揃え、5 構造体で統一
- obsws 経路 5 箇所 (`src/obsws/coordinator/` 2 箇所 + `src/obsws/source/` 3 箇所) をリテラル組立から `new()?` 呼びに置換し、エラーメッセージプレフィックスを `"invalid <module_snake_case> config: {e}"` で統一
- `start_*_processors` で encoder 起動前に `new()` 検証を済ませ、検証失敗時の encoder processor リークを防止
- `tests/{rtmp_publisher,rtmp_inbound_endpoint,rtmp_outbound_endpoint,srt_inbound_endpoint,rtsp_subscriber}_tests.rs` を 5 ファイル新設し、各 `*BuildError` バリアントと正常系の組み合わせを覆った
- obsws 経路 3 ファイルの `mod tests` に `is_source_startable_accepts_empty_input_url` と `build_record_source_plan_rejects_empty_input_url` を追加し、空文字弾きの責務分担を退行検知
- `docs/internals/processor_validation.md` を新規作成し、設計原則 (eager / lazy / 型保証 / Display 実装 / Options 集約) と新規 processor 追加時の確認事項を集約

### レビュー対応 (`/review-diff-code`)

レビューで指摘された致命的・重要を反映した:

- 致命的 (F1): docs/internals/ ノートから issue 番号 / ブランチ名 / commit hash を削除
- 重要 (W1): obsws coordinator の encoder 起動前に `new()` 検証を済ませる順序入れ替え
- 重要 (W2 / W3): 5 構造体本体と `new()` の doc コメントから実装詳細の列挙を削り、設計原則は `docs/internals/processor_validation.md` に集約
- 重要 (W4): tests の `let _ = endpoint;` 19 箇所を削除し、正常系を `Result<(), *BuildError>` + `?` パターンに置換
- 重要 (W6): `RtmpInboundEndpointOptions` / `RtmpOutboundEndpointOptions` の `cert_path` / `key_path` を Some にする正常系、および SRT で `key_length` / `tsbpd_delay_ms` を Some にする正常系を追加
- 重要 (W7): obsws/source 3 ファイルの `mod tests` に `is_source_startable` の空文字境界テストと `build_record_source_plan` の InvalidInput 検証を追加
- 重要 (W8): `SrtInboundEndpointOptions` フィールドの `//` を `///` に統一
- 重要 (W9): `options_default_max_buffered_frame_count_is_1000` テストを削除 (Default 実装と同じリテラルを書く二重宣言で退行検知価値が薄いため)
- 改善 (I3 / I4 / I5 / I8): テストコメントを「退行検知の対象」に書き直し、`..Default::default()` 化、reject テストでクロスペア (audio=None + video=Some) 採用、`*BuildError` doc 末尾の句点統一

### スコープ外で別 issue 候補とした項目

- `RtmpPublisher::run` 内の `parse_rtmp_url` 2 重呼び出し
- `RtmpInboundEndpointOptions` / `RtmpOutboundEndpointOptions` の空 PathBuf 検証
- 5 構造体の派生属性統一 (`Debug` / `Clone` 派生)
- obsws `is_source_startable` の空文字弾き (本 issue では `new()?` で弾く方針)
- テスト 5 ファイル間の重複を共通ヘルパ化 (review W5)
- `tests/test_<module>.rs` 命名規約と既存実態 (`*_tests.rs`) の SKILL 改定 (review 観点 3 改善)

### 検証

CI 同等の `cargo fmt --all --check` / `cargo check --workspace` / `cargo check --workspace --no-default-features` / `cargo clippy --workspace --all-targets -- --deny warnings` / `cargo clippy --workspace --no-default-features -- --deny warnings` / `cargo test --workspace` / `cargo test --workspace --no-default-features` をすべてパスした。`CHANGES.md` には記載しなかった (issue 本文で確定した方針通り)。
