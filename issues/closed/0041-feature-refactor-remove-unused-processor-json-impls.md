# JSON-RPC 削除で呼び出し元を失った processor 5 構造体の DisplayJson / TryFrom 実装を削除する

- Priority: Low
- Created: 2026-06-17
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-unused-processor-json-impls
- Polished: 2026-06-18

## 目的

`RtmpInboundEndpoint` / `RtmpOutboundEndpoint` / `RtmpPublisher` / `SrtInboundEndpoint` / `RtspSubscriber` の 5 構造体には `nojson::DisplayJson` と `TryFrom<nojson::RawJsonValue>` の手書き実装が残っている。これらは元々 hisui server サブコマンドの JSON-RPC `createRtmpInboundEndpoint` / `createRtmpOutboundEndpoint` / `createRtmpPublisher` / `createSrtInboundEndpoint` / `createRtspSubscriber` メソッド経由で構造体を組み立てるための引数パーサとして追加されたものだが、その JSON-RPC 機能が `feature/remove-json-rpc` ブランチ (PR #207, merge commit `d4170ed8`) で全削除されたことにより呼び出し元を失っている。現在は obsws coordinator が `crate::rtmp::publisher::RtmpPublisher { ... }` の直接フィールド代入で構造体を組み立てており、JSON 経由のシリアライズ / デシリアライズ経路は実コード上に存在しない。死活確認のうえ impl ブロックと連鎖 helper を削除し、対応する docs 段落と JSON-RPC を前提とした内部コメントも整理する。

## 現状

### 対象 impl 群

実コードでの位置 (2026-06-18 時点で再確認):

| ファイル | `impl DisplayJson` | `impl TryFrom<RawJsonValue>` |
| --- | --- | --- |
| `src/rtmp/inbound_endpoint.rs` | 225-241 | 243-286 |
| `src/rtmp/outbound_endpoint.rs` | 66-90 | 92-149 |
| `src/rtmp/publisher.rs` | 64-88 | 90-146 |
| `src/srt/inbound_endpoint.rs` | 358-388 | 390-433 |
| `src/rtsp/subscriber.rs` | 37-50 | 52-79 |

### 既に確認済みの調査結果

closed issue 0003 (`obsws JSON 命名規約` / merge commit `5378bd39`) のレビュー過程および本 issue polish 時の再 grep で確認済み:

- `grep -rn 'try_into()' src/ | grep -iE 'RtmpInboundEndpoint|RtmpOutboundEndpoint|RtmpPublisher|SrtInboundEndpoint|RtspSubscriber'` で対象 5 構造体への JSON 経由の生成呼び出しはゼロ
- `nojson::object(|f| ...)` 内で `f.value(&endpoint)` 等で構造体を JSON に埋め込む箇所もゼロ。`tracing::debug!` 等で構造体全体を Debug / JSON 表示する経路もゼロ (個別フィールドのみ参照)
- `src/obsws/state_file.rs`、obsws coordinator (`src/obsws/coordinator/output_rtmp.rs:282` / `output_stream.rs:359` / `source/srt_inbound.rs:26` 等)、`src/obsws/source.rs` の `ObswsSourceRequest` バリアントはすべて構造体を直接フィールド代入で受け取っており JSON 経由ではない
- `tests/` / `pbt/` / `examples/` / `testdata/` / `e2e-tests/` / `devtools/` のいずれにも参照ゼロ
- 5 構造体の impl ブロックに `#[cfg(test)]` / `#[cfg(feature = "...")]` 修飾はなく、間接利用経路はない

### 連鎖削除候補

impl 削除に追従して呼び出し元を失う helper:

- `src/srt/inbound_endpoint.rs:435-452` の `fn parse_optional_non_empty_string` — `TryFrom` (line 406, 407) でのみ使用
- `src/srt/inbound_endpoint.rs:454-474` の `fn parse_optional_key_length` — `TryFrom` (line 408) でのみ使用
- `src/srt/inbound_endpoint.rs:476-481` の `fn key_length_to_rpc_value` — `DisplayJson` (line 375) でのみ使用
- `src/rtsp/subscriber.rs:1188-1192` の `fn validate_input_url` — `TryFrom` (line 61) でのみ使用 (`run` 経路は `parse_rtsp_input_url` を直接呼ぶため不要)

連鎖削除に **含めない** helper:

- `src/srt/inbound_endpoint.rs:483-487` の `fn tsbpd_delay_duration_to_millis` — `DisplayJson` (line 379) だけでなく `SrtInboundEndpoint::endpoint_config` (line 303) でも `Duration → u16 ミリ秒` 変換に使われており impl 削除後も残す必要がある

### コメント整合修正対象

JSON-RPC 経路の存在を前提に書かれた内部コメントが残る。impl 削除と同 PR で整合させる:

- `src/srt/inbound_endpoint.rs:31` の `tsbpd_delay_ms` フィールドコメント `// TSBPD 遅延。JSON-RPC ではミリ秒の u16 で受け取り、内部では Duration で保持する。` の前半 (`JSON-RPC ではミリ秒の u16 で受け取り、`) を削除し、`// TSBPD 遅延。内部では Duration で保持し、SRT 接続オプションには `endpoint_config` で u16 ミリ秒に変換して渡す。` 等に書き換える

着手時に対象 5 ファイル全体を `grep -nE 'JSON-?RPC|json-?rpc|createRtmp|createSrt|createRtsp' src/rtmp/inbound_endpoint.rs src/rtmp/outbound_endpoint.rs src/rtmp/publisher.rs src/srt/inbound_endpoint.rs src/rtsp/subscriber.rs` で再点検し、追加で出てきたコメントも同じスコープで整合させる。

### docs 編集対象

`docs/obsws/json_naming.md:83` (章 4 末尾の独立段落):

```
hisui 内部の processor (RTMP / SRT / RTSP の Endpoint / Subscriber 等) が持つ独自 JSON フォーマット (obsws を経由しないキー) は obsws JSON プロトコルの境界外であり、本規約の対象外とする。
```

impl 削除後は「processor 独自 JSON フォーマット」自体が存在しなくなるため、この段落 1 行を **完全削除** する (代わりの段落は不要)。

## 設計方針

### 1. 死活確認 (commit しない作業ブランチ上のみ)

対象 impl 10 ブロックと **連鎖削除候補 helper 4 個** (`parse_optional_non_empty_string` / `parse_optional_key_length` / `key_length_to_rpc_value` / `validate_input_url`) を **すべて同時に** `#[cfg(any())]` で一時的に無効化する。impl 10 ブロックを単独で無効化すると helper 4 個が呼び出し元喪失で `dead_code` 警告に化け、`cargo clippy --workspace --all-targets -- --deny warnings` の `--deny warnings` で deny されるため、死活確認のシグナルが取れなくなる。

`tsbpd_delay_duration_to_millis` は `endpoint_config` (line 303) でも使われているため `#[cfg(any())]` の対象に **含めない** (含めると `endpoint_config` 側で型エラーになる)。

適用位置: 対象 impl ブロックと helper 関数定義の **直前 1 行** に `#[cfg(any())]` を追加する。合計で `#[cfg(any())]` 属性を 14 箇所 (impl 10 ブロック直前 + helper 4 関数直前) に追加する。属性自身の追加行数は 14 行で、配下のブロック全体が無効化される。

### 2. 検証

CI と同等のコマンドで通すこと:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`

`--features nvcodec` / `--features fdk-aac` ジョブはローカル環境依存が大きいため CI 任せでよい。

`#[cfg(any())]` 無効化下で **対象 impl / helper 由来以外** の型エラー・警告が一切出ないこと、特に `dead_code` 以外の警告が一切出ないことを確認する。出た場合は本 issue の前提 (呼び出し元不在) が崩れているので削除を中止し、後述の撤退条件に従う。

### 3. 実削除と docs / コメント整合

死活確認を通過したら、`#[cfg(any())]` 付与を巻き戻したうえで以下を 1 commit にまとめる:

- impl 10 ブロックと helper 4 個を実削除
- `src/srt/inbound_endpoint.rs:31` のコメント整合修正 (`### コメント整合修正対象` 参照)
- 着手時 grep で追加検出されたコメントの整合修正
- `docs/obsws/json_naming.md:83` の章 4 末尾段落 1 行を削除 (前後の空行も整える)

### 4. PR 提出前検証

設計方針 2 と同じコマンドを再度通す。加えて以下を確認:

- `cargo doc --no-deps` の警告数が着手時ベースラインを超えないこと。ベースラインは着手時に develop ブランチで `cargo doc --no-deps 2>&1 | grep -E '^warning:' | wc -l` を測り、close 時に本 issue ファイル末尾へ追記する `## 解決方法` 節に「`cargo doc --no-deps` 警告: 着手時 N → 完了時 M」の形で記録する

### 5. 撤退条件

死活確認段階で `dead_code` 以外の警告 / エラーが出た場合 (JSON 経由で構造体を扱う未把握の経路が見つかった場合) は、作業ブランチを push せず `#[cfg(any())]` 付与状態のままローカルで保持し、本 issue ファイルの `### 既に確認済みの調査結果` セクションに「再調査が必要」として状況を追記して polish-issue で改稿する。

## 完了条件

- 対象 5 ファイルから `impl nojson::DisplayJson` と `impl TryFrom<nojson::RawJsonValue>` の各 impl ブロックが削除されていること
- 連鎖削除候補 4 個 (`parse_optional_non_empty_string` / `parse_optional_key_length` / `key_length_to_rpc_value` / `validate_input_url`) が削除されていること。`tsbpd_delay_duration_to_millis` は `endpoint_config` で使い続けるため残ること
- `src/srt/inbound_endpoint.rs:31` のコメントから `JSON-RPC ではミリ秒の u16 で受け取り、` の文言が削除されていること
- `docs/obsws/json_naming.md` の章 4 末尾段落 (`hisui 内部の processor (...) は obsws JSON プロトコルの境界外であり、本規約の対象外とする。`) が削除されていること
- 以下の grep が **すべて 0 件** (流派は GNU grep / BSD grep 共通の `-rnE` ERE で統一):
  - `grep -rnE 'impl nojson::DisplayJson for (RtmpInboundEndpoint|RtmpOutboundEndpoint|RtmpPublisher|SrtInboundEndpoint|RtspSubscriber)' src/`
  - `grep -rn 'TryFrom<nojson::RawJsonValue' src/rtmp/ src/srt/ src/rtsp/`
  - `grep -rn 'hisui 内部の processor' docs/`
  - `grep -rnE 'JSON-?RPC|json-?rpc' src/rtmp/ src/srt/ src/rtsp/`
- `cargo fmt --all --check` / `cargo check --workspace` / `cargo check --workspace --no-default-features` / `cargo clippy --workspace --all-targets -- --deny warnings` / `cargo clippy --workspace --no-default-features -- --deny warnings` / `cargo test --workspace` が CI と同等のコマンドですべて通ること
- `cargo doc --no-deps` の警告数が着手時ベースラインを超えないこと (記録方法は設計方針 4)

## CHANGES.md について

`CHANGES.md` には **追記しない**。

- `docs/obsws/json_naming.md` の編集分は、shiguredo-changelog 規約「`.rst` / `.md` ファイルの変更は変更履歴に反映しないこと (コード変更と同時に行った場合も、ドキュメント変更分はエントリに含めない)」に従い、CHANGES.md に反映しない
- impl / helper 削除分は、利用者から見える挙動・CLI / env / stdout・公開 API・依存関係はいずれも変化しない内部 dead code 整理のため、closed 0036 (`feature/refactor-japanize-comment-terms`) / closed 0022 (`feature/refactor-fmp4-reader-naming`) と同じ先例に倣う

## 関連

- closed PR #207 (`feature/remove-json-rpc` / merge commit `d4170ed8`): 本 issue 対象の impl が呼び出し元を失った直接の契機 (`MediaPipelineHandle` から JSON-RPC 依存除去は `d8151946`)

## 解決方法 (2026-06-18)

`feature/refactor-remove-unused-processor-json-impls` で次を実装した。2 コミット、6 ファイル +2/-411 行。

- `src/rtmp/inbound_endpoint.rs` / `src/rtmp/outbound_endpoint.rs` / `src/rtmp/publisher.rs` / `src/srt/inbound_endpoint.rs` / `src/rtsp/subscriber.rs` から `impl nojson::DisplayJson` と `impl TryFrom<nojson::RawJsonValue>` の各 impl ブロック (合計 10 ブロック) を削除した。
- 連鎖 helper 4 個 (`parse_optional_non_empty_string` / `parse_optional_key_length` / `key_length_to_rpc_value` / `validate_input_url`) を削除した。`tsbpd_delay_duration_to_millis` は `endpoint_config` で使い続けるため残した。
- `src/srt/inbound_endpoint.rs:31` の `tsbpd_delay_ms` フィールドコメントから旧 JSON-RPC 言及を除去し、周辺フィールドの密度に揃えて `// TSBPD 遅延。` に簡潔化した。
- `docs/obsws/json_naming.md` 章 4 末尾の processor 独自 JSON フォーマットに関する段落 1 行を削除した。
- 死活確認は impl 10 ブロックと helper 4 個を **すべて同時に** `#[cfg(any())]` で無効化したうえで CI 同等の `cargo check --workspace` / `cargo check --workspace --no-default-features` / `cargo clippy --workspace --all-targets -- --deny warnings` / `cargo clippy --workspace --no-default-features -- --deny warnings` / `cargo test --workspace` がすべてパスすることを確認した。helper 連鎖の `dead_code` 警告すら出ず呼び出し元不在を実証した。
- 完了条件の grep 4 種 (`impl nojson::DisplayJson for ...` / `TryFrom<nojson::RawJsonValue` / `hisui 内部の processor` in docs / `JSON-?RPC|json-?rpc|createRtmp|createSrt|createRtsp` in 対象 5 ファイル) すべて 0 件達成。
- `cargo fmt --all --check` / CI 同等の cargo コマンドすべてパス、`cargo doc --no-deps` 警告数 着手時 4 → 完了時 4 (差分 0)。
- `CHANGES.md` には記載しなかった (shiguredo-changelog 規約「`.rst` / `.md` ファイルの変更は変更履歴に反映しない」と内部 dead code 整理のため、closed 0036 / 0022 と同じ先例に倣った)。
- `/review-diff-code` で重要 1 件 (フィールドコメントの密度乖離) を検出し、コメント簡潔化コミットで解消した。残った重要・改善指摘 5 件はすべて本 issue のスコープ外で、別 issue 起票候補として実装者が記録済み (PROTOCOL_STATUS.md 旧 JSON-RPC 名残整理、5 構造体 validation 集約方針、`tsbpd_delay_duration_to_millis` のエラー文言、`endpoint_config()` テスト追加、obsws output coordinator テスト追加)。
