# 削除済み pipeline サブコマンドの残骸として残った processor 構造体の DisplayJson / TryFrom 実装を削除する

- Priority: Low
- Created: 2026-06-17
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-unused-processor-json-impls
- Polished:

## 目的

CHANGES.md `## develop` の `[CHANGE] 実験的な pipeline サブコマンドを削除する` で削除された pipeline サブコマンドは、processor のグラフを JSON 設定ファイル (recipe) で組み立てる機能だった。各 processor 構造体は recipe JSON でインスタンス化される必要があったため、`nojson::DisplayJson` と `TryFrom<nojson::RawJsonValue>` の実装を持っていた。

pipeline サブコマンドが削除されたことで、これらの impl は呼び出し元を失っていると見られる。コードベースに死にコードとして残ったままだと:

- `docs/obsws/json_naming.md` 章 4.7 が「obsws JSON 規約と無関係な camelCase キーが残る」状態を許容する文面を維持し続ける必要がある (本来そんなキーは存在しないのに)
- 新規プロセッサ追加時に「DisplayJson / TryFrom を実装すべきか」の判断材料を曖昧にする

死にコードであることを最終確認したうえで、impl ブロックと連鎖 helper を削除する。

## 優先度根拠

Low。

- 機能影響なし: 削除済み pipeline サブコマンドが利用していたコード経路のみが対象で、現状の機能 (obsws / compose / inspect 等) には一切影響しない
- 緊急度なし: 死にコードが残っていても直接の害は無い
- ただしコードベース clean-up の延長として、obsws JSON 命名規約 (issue 0003 で整理) の保守性を高める副次的価値がある

## 現状

### 対象 impl 群

issue 0003 (obsws JSON 命名規約統一) のレビュー中に確認した範囲:

- `src/rtmp/inbound_endpoint.rs:225-285` — `RtmpInboundEndpoint::DisplayJson` / `TryFrom<RawJsonValue>`
- `src/rtmp/outbound_endpoint.rs:66-130` — `RtmpOutboundEndpoint` 同等 impl
- `src/rtmp/publisher.rs:64-130` — `RtmpPublisher` 同等 impl
- `src/srt/inbound_endpoint.rs:358-432` — `SrtInboundEndpoint` 同等 impl
- `src/rtsp/subscriber.rs:36-77` — `RtspSubscriber` 同等 impl

これらの impl 内で扱われている camelCase キー (`outputAudioTrackId` / `outputVideoTrackId` / `inputAudioTrackId` / `inputVideoTrackId` / `keyLength` / `tsbpdDelayMs` / `certPath` / `keyPath` 等) は obsws / obsdc の JSON プロトコルには流れず、削除済み pipeline サブコマンドの recipe 形式専用だった。

### 既に確認済みの調査結果

issue 0003 ブランチで以下を確認:

- `grep -rn 'try_into\(\)' src/` で対象 5 構造体を JSON から生成する呼び出し元はゼロ
- `nojson::object(|f| ...)` 内で `f.value(&endpoint)` のように構造体を埋め込む箇所もゼロ
- `tracing::debug!` 等のログでも構造体全体を Debug / JSON 表示する経路はなく、個別フィールド (`{addr}` 等) のみ
- `src/obsws/state_file.rs` と obsws coordinator は構造体を直接フィールド代入で受け取っており、JSON 経由ではない
- `src/obsws/source/rtmp_inbound.rs` / `srt_inbound.rs` / `rtsp_subscriber.rs` も `ObswsSourceRequest::CreateRtmpInboundEndpoint { endpoint, .. }` 等で構造体を直接渡している
- examples / testdata / e2e-tests でも参照ゼロ

### 連鎖削除候補

`src/srt/inbound_endpoint.rs` 内の `parse_optional_non_empty_string` / `parse_optional_key_length` / `key_length_to_rpc_value` / `tsbpd_delay_duration_to_millis` などの helper は、上記 `TryFrom` / `DisplayJson` 内でのみ使われている可能性が高い。impl 削除に追従して取り除く対象。

### 未確認事項

本 issue 着手時に検証する:

- `#[cfg(feature = "...")]` / `#[cfg(test)]` 経由の間接利用がないか
- `Debug for SrtInboundEndpoint` などの derive が `DisplayJson` に依存していないか
- 一時的に impl ブロックを `#[cfg(any())]` で無効化してビルド・テストを通すことによる死活確認
- docs / コメントから recipe 形式への参照が残っていないか

## 設計方針

1. **死活確認を最優先**: 対象 impl ブロックを `#[cfg(any())]` で一時的に無効化し、`cargo check --all-features --tests --benches` と `cargo clippy --all-targets --all-features -- -D warnings` を実行する。pub で隠れた呼び出し元が無いことを確認する
2. 死活確認後、対象 impl ブロックを削除する
3. 連鎖削除候補の helper も呼び出し元喪失で消えるので追従削除する
4. `docs/obsws/json_naming.md` 章 4.7 を更新する: 「processor 独自キーは規約対象外」段落を削除し、「これらのファイルでは obsws settings 由来キーが snake_case で流れる」程度のシンプルな文面に戻す

## 完了条件

- 対象 5 ファイルから `impl nojson::DisplayJson` と `impl TryFrom<nojson::RawJsonValue>` の各 impl ブロックが削除されていること
- 連鎖削除候補の helper (`parse_optional_non_empty_string` / `parse_optional_key_length` / `key_length_to_rpc_value` / `tsbpd_delay_duration_to_millis` 等) が呼び出し元と共に削除されていること
- `cargo check --all-features --tests --benches` / `cargo test --all` / `cargo clippy --all-targets --all-features -- -D warnings` がすべて通ること
- `docs/obsws/json_naming.md` 章 4.7 から「processor 独自キーは規約対象外」の記述が削除されていること

### CHANGES.md

`## develop` に追記しない。削除済み pipeline サブコマンドの内部実装を整理するのみで、利用者から見える挙動・公開 API・依存関係は一切変化しないため。

## 解決方法

1. **死活確認**: 対象 impl 群 (`impl DisplayJson` / `impl TryFrom`) を `#[cfg(any())]` で一時的に無効化し、`cargo check --all-features --tests --benches` と `cargo clippy --all-targets --all-features -- -D warnings` を実行する。呼び出し元の不在を確認する
2. impl ブロックを削除する。連鎖 helper も削除する
3. `docs/obsws/json_naming.md` 章 4.7 を更新する
4. 全テスト・clippy・fmt を通す
