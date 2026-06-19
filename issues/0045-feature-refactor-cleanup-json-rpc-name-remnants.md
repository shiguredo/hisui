# docs/obsws/PROTOCOL_STATUS.md と SRT エラー文言に残る旧 JSON-RPC メソッド名・キー名を整理する

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-cleanup-json-rpc-name-remnants
- Polished: 2026-06-19

## 目的

closed PR #207 (`feature/remove-json-rpc` / merge commit `d4170ed8`) と closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`) の整理から漏れた、JSON-RPC 時代の名残 2 種類を 1 issue / 1 PR で片付ける。

- docs 側: `docs/obsws/PROTOCOL_STATUS.md` の `### Stream` / `### Record` セクション内 NOTE 4 行 (line 342, 343, 370, 371) に削除済み JSON-RPC メソッド名 5 種 (`createPngFileSource` / `createVideoEncoder` / `createRtmpOutboundEndpoint` / `createVideoMixer` / `createMp4Writer`) が引用されたまま残る。
- コード側: `src/srt/inbound_endpoint.rs` の `endpoint_config()` 内 2 箇所 (line 293 / line 361) のエラー文言が旧 JSON-RPC 入力キー名 `keyLength` / `tsbpdDelayMs` (camelCase) を文字列リテラルで引用したまま残る。

両者は closed 0041 の解決方法末尾で「後追い起票候補」として記録された 5 件のうちの 2 件で、PR #207 で削除された JSON-RPC メソッド名 / 入力キー名の `create[A-Z][a-zA-Z]+` 系および camelCase キー残骸の grep で同時発見される動機共通の名残。

## 優先度根拠

Low。利用者影響ゼロの内部命名衛生のみ。

- docs 側は obsws WebSocket クライアントから観測不能な「内部実装 processor 名」の引用を消すだけで、obsws API の挙動・公開キー名・依存関係は変化しない。
- コード側のエラー文言は `spawn_processor` 配下の `tracing::error!` ログのみで露出し obsws WebSocket クライアントには Result として戻らない (詳細は `### エラー伝播経路と「dead path」前提` 参照)。

## 現状

### docs 側 (PROTOCOL_STATUS.md 4 行)

`grep -rnE 'createPngFileSource|createVideoEncoder|createVideoMixer|createRtmpOutboundEndpoint|createAudioMixer|createAudioEncoder|createMp4Writer' src/ docs/` 実行結果 (2026-06-19 時点):

- `docs/obsws/PROTOCOL_STATUS.md:342` — `- NOTE: 内部では \`createPngFileSource\` -> \`createVideoEncoder\` -> \`createRtmpOutboundEndpoint\` を起動する`
- `docs/obsws/PROTOCOL_STATUS.md:343` — `- NOTE: 複数映像入力時は \`createVideoMixer\` を追加で起動する`
- `docs/obsws/PROTOCOL_STATUS.md:370` — `- NOTE: 内部では \`createPngFileSource\` -> \`createVideoEncoder\` -> \`createMp4Writer\` を起動する`
- `docs/obsws/PROTOCOL_STATUS.md:371` — `- NOTE: 複数映像入力時は \`createVideoMixer\` を追加で起動する`

`src/` 配下にこれらメソッド名のヒットはゼロ。`createAudioMixer` / `createAudioEncoder` は `src/ docs/` いずれにもヒット 0 件 (将来の再混入検知用に grep に含めるだけ)。

該当 4 行は `## RequestType 実装状況` 配下の `### Stream` (line 321 付近〜) と `### Record` (line 349 付近〜) のうち、`StartStream` / `StartRecord` 項目内に挟まっている。前後の他 NOTE 群は **外向き挙動を主として説明し、必要に応じて内部実装識別子を補足参照する** スタイルで統一されている (例: line 330 `RTMP outbound endpoint の送信バイト数を返す`、line 334 `stream encoder の \`total_output_video_frame_count\` を返す`、line 346 `内部で起動した stream 用 processor を停止する`)。対象 4 行のみが外向き挙動の説明を欠き、内部実装 processor 名の列挙だけで構成されている点が既存 NOTE 群と異なる。

### コード側 (camelCase エラー文言 2 行)

`grep -rnE 'tsbpdDelayMs|keyLength' src/` 実行結果:

- `src/srt/inbound_endpoint.rs:293` — `"keyLength requires passphrase to be specified"`
- `src/srt/inbound_endpoint.rs:361` — `format!("tsbpdDelayMs must be <= {}", u16::MAX)` の中の文字列リテラル `"tsbpdDelayMs must be <= {}"`

line 293 のエラーは `SrtInboundEndpoint::endpoint_config()` (line 290-307) 内で直接生成される。line 361 のエラーは別関数 `tsbpd_delay_duration_to_millis` (line 358-362) 内で生成され、`endpoint_config()` line 303 の呼び出し経由で伝播する。発火経路の起点はいずれも `endpoint_config()` のみ。JSON-RPC 経路の `TryFrom<RawJsonValue>` は closed 0041 で削除済みで、`keyLength` / `tsbpdDelayMs` という camelCase キーが外部 JSON として流れる経路は存在しない。

同ファイル内の他エラー文言は「英語動詞句 + 必要なら内部識別子 (snake_case)」のハイブリッド型で統一されている (例: line 131 `"invalid input_url: {e}"`、line 136 `"invalid bind address: {e}"`、line 275 `"failed to process SRT packet: {e}"`)。line 293 / line 361 の 2 箇所は内部識別子を camelCase で露出する点のみが外れ値。

### エラー伝播経路と「dead path」前提

`endpoint_config()` を呼ぶ唯一の経路は `SrtInboundEndpoint::run()` 内 (line 132)。`run()` は `src/media_pipeline.rs:622-631` の `spawn_processor` 配下の `tokio::spawn` で実行され、`Err(e)` は呼び出し側に Result として戻らず、`processor_failed: AtomicBool` 更新 + `tracing::error!` ログのみで露出する。

さらに `SrtInboundEndpoint` を生成する唯一の現存経路 `src/obsws/source/srt_inbound.rs:26-44` の `build_record_source_plan` は `key_length: None` / `tsbpd_delay_ms: None` をハードコードしているため、line 291 の `key_length.is_some() && passphrase.is_none()` 分岐と line 302-303 の `tsbpd_delay_duration_to_millis` 呼び出しはいずれも現状到達不能で、line 293 / line 361 のエラーは現状 dead path。将来 obsws coordinator 側で `ObswsSrtInboundSettings` に `key_length` / `tsbpd_delay_ms` を渡せるよう拡張する場合に初めて発火する。

本 issue の動機は「dead path 解消」ではなく **camelCase → snake_case の内部命名衛生** (closed 0041 のフィールドコメント整合と同じ哲学)。利用者影響ゼロの根拠は dead path ではなく `tracing::error!` ログのみで露出するエラー伝播経路の方に依存する (dead path が将来解消されても伝播経路は変わらない)。

将来 dead path を解消する PR (`ObswsSrtInboundSettings` に `key_length` / `tsbpd_delay_ms` を追加して obsws coordinator から渡せるよう拡張する変更) では、両 validation 経路が初めて活性化するため、`endpoint_config()` 単体テスト (closed 0041 line 150 で「後追い起票候補」として記録された未起票項目) を併せて追加して回帰検知力を担保すること。

### スコープ外

- `endpoint_config()` 内の validation 責務分担 (どこで弾くか、何を弾くか、構造体側 / coordinator 側 / 型レベルへの集約) は open issue 0046 のスコープ。本 issue はエラー文言の literal だけを書き換え、validation のロジック (条件式・引数・発火箇所) は変更しない。

## 設計方針

### 1. docs/obsws/PROTOCOL_STATUS.md 4 行を削除する

該当 4 行 (line 342, 343, 370, 371) を NOTE 行ごと削除する。書き換え案 (内部 processor 名ベースに書き直す) は不採用。

- 当該 4 行は外向き挙動の説明を欠き内部 processor 名の列挙のみで構成されており、`### Stream` / `### Record` の他 NOTE と異なるスタイル (`### 現状 → docs 側` 参照)。
- 書き換え案で「PNG file source → stream encoder → rtmp_publisher を起動する」のように現実装の processor 名に揃えると、`StartStream` の内部 processor 名 `rtmp_publisher` (push 側、`src/obsws/coordinator/output_stream.rs:102` で命名) と `### 独自 Output` 配下の `rtmp_outbound` (pull 側 output) との混同を新たに招く。なお `rtmp_publisher` / `rtmp_outbound` の文字列は現状 PROTOCOL_STATUS.md 内には登場しないため、書き換え案を採るとこれらの内部識別子を新たに docs に持ち込むことになる。

### 2. `src/srt/inbound_endpoint.rs` の camelCase 識別子 2 箇所を snake_case 化する

**camelCase 識別子部分のみを snake_case 化し、語順・前後の英語動詞句・`format!` 引数・条件式・発火箇所は変更しない**:

- line 293: `"keyLength requires passphrase to be specified"` → `"key_length requires passphrase to be specified"`
- line 361: `format!("tsbpdDelayMs must be <= {}", u16::MAX)` → `format!("tsbpd_delay_ms must be <= {}", u16::MAX)`

書き換え時の禁止事項 (上記 2 行に対して):

- 主述反転 (例: `passphrase requires key_length` / `key_length must be specified with passphrase`) は行わない。本 issue は語順整理を扱わない (語順整理は 0046 で `endpoint_config()` の validation を再配置する際に併せて検討する別件)。
- `tsbpd_delay_ms` を `tsbpd_delay` に短縮しない (同ファイル line 105 に `SrtEndpointConfig::tsbpd_delay: u16` が別途存在し紛らわしくなる)。書き換え後の `tsbpd_delay_ms must be <= 65535` は「`SrtInboundEndpoint::tsbpd_delay_ms: Option<Duration>` (line 32) の値がミリ秒換算で u16 範囲を超える」を意味し、変換後の `SrtEndpointConfig::tsbpd_delay: u16` (line 105) ではなく変換前のフィールドを指す。
- `SRT` などのプロトコル名プレフィックスや `_value` / `_millis` 等の修飾は追加しない (同ファイル他エラー文言と整合させる)。

closed 0036 (コメント日本語化) との関係: 0036 は line 50 で文字列リテラル全般 (`crate::Error::new("...")` / `format!("...")` のエラー文字列を含む) を対象外と明示しており、本 issue が触る 2 箇所はまさに 0036 のスコープ外 → 競合なし。

## 完了条件

### grep 0 件

GNU grep / BSD grep 共通の `-rnE` ERE で統一。closed 0041 と同流派。検索パスは `src/ docs/ tests/ pbt/ examples/` (本 issue 本文が旧キー名を引用する性質上、`issues/` 配下は最初から検索対象に含めない)。

- 設計方針 §1 (docs 4 行削除) の検証: `grep -rnE 'createPngFileSource|createVideoEncoder|createVideoMixer|createRtmpOutboundEndpoint|createAudioMixer|createAudioEncoder|createMp4Writer' src/ docs/ tests/ pbt/ examples/` が **0 件**
- 設計方針 §2 (camelCase エラー文言 2 箇所の snake_case 化) の検証: `grep -rnE 'tsbpdDelayMs|keyLength' src/ docs/ tests/ pbt/ examples/` が **0 件**

### cargo コマンド (CI と同等)

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`

`cargo test --workspace` は本 issue が書き換える 2 リテラルを assert する既存テストがないため (`tests/` `pbt/` を `grep` した範囲では 0 件)、本 issue の変更を **直接** 検証はしない。CI と同等の regression check として走らせる目的。新規テスト追加は不要。

`--features nvcodec` / `--features fdk-aac` ジョブはローカル環境依存のため CI 任せでよい。

### cargo doc 警告数

`cargo doc --no-deps` (デフォルト features) の警告数が着手時ベースラインを超えないこと。

ベースラインは `git checkout develop && git pull` 直後 (作業ブランチを切る前) に `cargo doc --no-deps 2>&1 | grep -E '^warning:' | wc -l` を測り、close 時に本 issue ファイル末尾へ追記する `## 解決方法` 節に「`cargo doc --no-deps` 警告: 着手時 N → 完了時 M」の形で記録する (closed 0041 と同手順)。`--all-features` / `--no-default-features` 版は対象外 (本 issue は docstring を一切触らないため警告数増減の余地が極めて狭く、デフォルト features のみで十分)。

## CHANGES.md について

`CHANGES.md` には追記しない。

- `docs/obsws/PROTOCOL_STATUS.md` の編集は shiguredo-changelog 規約「`.rst` / `.md` ファイルの変更は変更履歴に反映しない」に従う。
- `src/srt/inbound_endpoint.rs` のエラー文言書き換えは `tracing::error!` ログのみで露出し利用者影響ゼロ。closed 0036 / closed 0022 / closed 0041 と同じ「機能・互換性に影響しない内部整合は CHANGES.md に記載しない」先例に倣う。

## 関連

- closed PR #207 (`feature/remove-json-rpc` / merge commit `d4170ed8`): 本 issue の名残が生じた契機 (hisui server サブコマンドの JSON-RPC 機能全削除)。
- closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`): 5 processor 構造体の `DisplayJson` / `TryFrom` 削除。解決方法末尾 (line 150) で本 issue を含む 5 件の後追い起票候補が記録された。本 issue (0045) と open 0046 で 3 件を拾い、残る 2 件 (`endpoint_config()` テスト追加 / obsws output coordinator テスト追加) の起票管理は本 issue のスコープ外。
- open issue 0046 (`feature/refactor-clarify-processor-validation-boundary`): 5 構造体の validation 責務分担を確定する issue。本 issue は validation のロジックは触らずリテラル文字列の内部命名衛生のみを扱うため、0046 と並行進行可能。`src/srt/inbound_endpoint.rs` の `endpoint_config()` (line 290-307) と `tsbpd_delay_duration_to_millis` (line 358-362) の周辺で先後に応じて trivial conflict は出るが、文言整合 (本 issue) と validation 移動 (0046) は意図が独立しているため機械的に解消できる。
