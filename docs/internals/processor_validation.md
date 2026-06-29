# `processor` 構造体の validation 責務分担

外部入出力を担う processor 構造体について、フィールドの不変条件をどこで保証するか（コンストラクタで eager に弾くか、`run()` 等で lazy に弾くか、型システムで静的に保証するか）の責務分担と判断基準をまとめます。

具体的なバリアント名・関数名・行番号といった実装詳細は変化するため本ノートでは扱いません。最新の事実はソースコードを参照してください。

## 設計原則

1. **暴露面を構造的に塞ぐ**
   - 構造体本体のフィールドは `pub(crate)` 以下に格下げし、コンストラクタ `new()` を唯一の組立経路とする
   - struct 自体は `pub` のまま維持し、`pub enum` の payload 等の利用箇所と整合させる
   - `#[non_exhaustive]` は使わない（`shiguredo-rust` 規約）

2. **検証は eager と lazy を明示的に分ける**
   - eager（コンストラクタ）: フィールド単独で判定可能で、誤った値を後段に流すと診断が遠くなるもの
   - lazy（`run()` または専用関数）: 別フィールドや実行時状態に依存する組み合わせ、外部仕様への準拠など、組立時点では判定が過剰になるもの

3. **型レベル保証を優先する**
   - 1 以上を要求する整数は `NonZeroUsize` 等で型化する
   - 型で表せる不変条件は実行時検証より型を優先する

4. **自前のエラー型を持ち、`Display` を実装する**
   - `*BuildError` enum を `#[derive(Debug)]` + `impl Display` で表現する
   - `Clone` 派生と `std::error::Error` 実装は不要（`?` 経由で即時伝播し、`From<E> for crate::Error` 自動変換も採用しないため）

5. **複数オプションを集約する**
   - フィールド数が増えるときは `*Options` 構造体に切り出し、`new()` の引数を増やしすぎない（clippy の `too_many_arguments` 回避と既存パターンへの整合）

## eager と lazy の振り分け基準

| 性質 | どこで検証するか | 例 |
|---|---|---|
| 単一フィールドで判定可能な非空性 | eager（コンストラクタ） | URL 文字列、stream 識別子 |
| 「少なくとも片方が必要」のような複数フィールドの組み合わせ | eager（コンストラクタ） | audio / video track_id のどちらか必須 |
| 型レベルで表現可能な範囲制約 | 型システム | `NonZeroUsize` で `>= 1` 保証 |
| 別フィールドや実行時状態に依存する組み合わせ | lazy | TLS 有効時のみ意味を持つフィールドの組合せ |
| 外部仕様への準拠（URL 構文、外部 crate API） | lazy | URL parser、外部プロトコルのハンドシェイク |
| 環境依存（ファイル存在等） | lazy、もしくは検証しない | 証明書ファイルの実在性 |

eager に寄せすぎると組立時点で実行時条件を持ち込むことになり責務が肥大化する。lazy に寄せすぎると不正値が後段まで届いて診断が遠くなる。両者の境界を「フィールド単独で完結するか」「外部状態に依存するか」で切り分ける。

## obsws 経路からの組立

obsws 経由で processor 構造体を組み立てる箇所は、必ず `new()?` を経由する。`*BuildError` を obsws のエラー型に変換する際は `Display` 経由（`format!("{e}")`）で文字列化し、文脈プレフィックスを付けてユーザー可読なメッセージにする。

`is_source_startable` のような事前判定関数は、ここでは空文字を含めた厳密な検証を持たせず、最終的な検証は `new()?` 段階で行う方針とする。事前判定と最終判定の責務を意図的に分けることで、事前判定の肥大化を防ぐ。

## 新規 processor 構造体を追加するときの確認事項

1. 構造体本体のフィールドを `pub(crate)` 化したか
2. `pub fn new(...) -> Result<Self, *BuildError>` を提供したか
3. 検証項目を「eager / lazy / 型」のどれに割り当てたか説明できるか
4. `*BuildError` は `Debug` 派生と `Display` 実装のみで構成したか（`Clone` と `std::error::Error` は不要）
5. 整数の範囲制約は型（`NonZeroUsize` 等）で表現したか
6. フィールドが多い場合は `*Options` 構造体に集約したか
7. obsws 経路から組み立てる場合は `new()?` を経由し、`Display` 経由のエラー変換で文脈プレフィックスを付けたか
8. integration test で正常系および各 `*BuildError` バリアントを覆ったか

## 実例: `RtmpPublisher`

`src/rtmp/publisher.rs` を例に、設計原則がどう適用されているかを示す。

- 本体フィールドは `pub(crate)`、`RtmpPublisher::new()` が唯一の組立経路（原則 1）
- `output_url` の非空、`stream_name` 指定時の非空、track_id の少なくとも片方必須は `new()` で eager 検証（原則 2）
- URL 構文妥当性は `run()` 冒頭の `parse_rtmp_url` で lazy 検証（原則 2）
- `RtmpPublisherOptions::max_buffered_frame_count` は `NonZeroUsize` で型保証し、`tokio::sync::mpsc::channel(0)` の panic を構造的に排除（原則 3）
- `RtmpPublisherBuildError` は `Debug` 派生 + `Display` 実装のみ（原則 4）
- obsws 経路は `RtmpPublisher::new()?` を呼び、`Display` 経由で文脈プレフィックス付きエラーに変換（obsws 経路の項）

他 4 構造体（`RtmpInboundEndpoint` / `RtmpOutboundEndpoint` / `SrtInboundEndpoint` / `RtspSubscriber`）も同じ原則で構築されている。`SrtInboundEndpoint` のように引数が多くなる場合は `SrtInboundEndpointOptions` に集約している（原則 5）。

## 関連

- 5 構造体本体: `src/rtmp/` / `src/srt/` / `src/rtsp/` 配下
- obsws 経路の組立点: `src/obsws/coordinator/` および `src/obsws/source/`
- `crate::Error` の設計方針: `src/error.rs`
- [`sample_entry_invariant.md`](sample_entry_invariant.md): writer 入口の sample_entry 不変条件
