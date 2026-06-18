# docs/obsws/PROTOCOL_STATUS.md と SRT エラー文言に残る旧 JSON-RPC メソッド名・キー名を整理する

- Priority: Low
- Created: 2026-06-18
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-cleanup-json-rpc-name-remnants
- Polished:

## 目的

PR #207 (`feature/remove-json-rpc` / merge commit `d4170ed8`) で hisui server サブコマンドの JSON-RPC 機能 (`createXxx` メソッド群) が全削除されたが、当時の整理から漏れた 2 種類の名残が現在も残っており、コード / docs と実 API の乖離を生んでいる。closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`) のレビューで本コミットの責務外と判定された 2 件を統合して整理する。

- `docs/obsws/PROTOCOL_STATUS.md` の `NOTE` 4 行に削除済みメソッド名 (`createPngFileSource` / `createVideoEncoder` / `createRtmpOutboundEndpoint` / `createVideoMixer` / `createMp4Writer`) が残存
- `src/srt/inbound_endpoint.rs:361` の `tsbpd_delay_duration_to_millis` のエラー文言が旧 JSON-RPC キー名 `tsbpdDelayMs` (camelCase) を参照。本ファイルから JSON-RPC 経路の `TryFrom<RawJsonValue>` が closed 0041 で削除されたため、`tsbpdDelayMs` キー名は外部に出ない (Rust フィールド名は `tsbpd_delay_ms`)

## 優先度根拠

Low。

- 機能影響なし。利用者から見える挙動・公開 API・依存関係はいずれも変化しない
- 内部命名の整合性回復のみ (PR #207 と closed 0041 で消したつもりの残骸の追従)
- ただし新規参照時に「`createXxx` メソッドが現存するのか」「`tsbpdDelayMs` キーが JSON に流れているのか」を読み手に誤読させる可能性があり、放置するほど寿命が延びるので早期に潰す価値はある

## 現状

### docs 側 (PROTOCOL_STATUS.md 4 行)

`grep -rnE 'createPngFileSource|createVideoEncoder|createVideoMixer|createRtmpOutboundEndpoint|createAudioMixer|createAudioEncoder|createMp4Writer' src/ docs/` 実行結果 (2026-06-18 時点):

- `docs/obsws/PROTOCOL_STATUS.md:342` — `- NOTE: 内部では \`createPngFileSource\` -> \`createVideoEncoder\` -> \`createRtmpOutboundEndpoint\` を起動する`
- `docs/obsws/PROTOCOL_STATUS.md:343` — `- NOTE: 複数映像入力時は \`createVideoMixer\` を追加で起動する`
- `docs/obsws/PROTOCOL_STATUS.md:370` — `- NOTE: 内部では \`createPngFileSource\` -> \`createVideoEncoder\` -> \`createMp4Writer\` を起動する`
- `docs/obsws/PROTOCOL_STATUS.md:371` — `- NOTE: 複数映像入力時は \`createVideoMixer\` を追加で起動する`

`src/` 配下にこれらメソッド名のヒットはゼロで、すべて PR #207 で削除済み。

### コード側 (tsbpdDelayMs エラー文言 1 行)

`grep -rn 'tsbpdDelayMs' src/` 実行結果:

- `src/srt/inbound_endpoint.rs:361` — `.map_err(|_| crate::Error::new(format!("tsbpdDelayMs must be <= {}", u16::MAX)))`

このエラーは `tsbpd_delay_duration_to_millis` 関数 (closed 0041 で `SrtInboundEndpoint::endpoint_config` 用に残された helper) から発生する。エラー伝播先は obsws coordinator の SRT 接続セットアップ経路で、利用者には Rust フィールド名 `tsbpd_delay_ms` の値が範囲外であることが分かる文言が望ましい (JSON 経路自体は closed 0041 で消滅済み)。

### 関連 docs / コメント

`docs/obsws/PROTOCOL_STATUS.md` の章構造は PR #207 当時の「JSON-RPC で何ができるか / 何が未実装か」のマトリクスを元にしているため、削除済みメソッド名を引用しないと意味が壊れる NOTE 行はない (周辺 NOTE 群は obsws WebSocket の挙動を説明していて、`createXxx` 言及は当該 4 行に限定される)。

## 設計方針

### 1. docs/obsws/PROTOCOL_STATUS.md 4 行の整理

各 NOTE 行を以下のいずれかに書き換える (実装時に着手者が選択):

- 案 A: 内部プロセッサ名ベースに書き換える (例: 「内部では PNG 入力 → 動画エンコーダ → RTMP 出力プロセッサを起動する」)
- 案 B: NOTE 行自体を削除する (元々 JSON-RPC 内部実装の説明だったため、obsws WebSocket 利用者には不要な情報の可能性が高い)

着手時に PROTOCOL_STATUS.md の前後文脈 (章 6 / 章 7 付近) を読んで、利用者に伝える価値がある情報か判断する。価値がなければ案 B を選ぶ。

### 2. tsbpd_delay_duration_to_millis のエラー文言書き換え

`src/srt/inbound_endpoint.rs:361` を以下のいずれかに書き換える:

- 案 A: 内部フィールド名ベース — `"tsbpd_delay_ms must be <= {}"`
- 案 B: 英語平文 — `"SRT TSBPD delay must be at most {} ms"`

着手者の判断で、obsws coordinator の他のエラー文言の書きぶり (`src/obsws/coordinator/` 内の Error::new 引数) と整合する方を選ぶ。

## 完了条件

- 以下の grep が **すべて 0 件**:
  - `grep -rnE 'createPngFileSource|createVideoEncoder|createVideoMixer|createRtmpOutboundEndpoint|createAudioMixer|createAudioEncoder|createMp4Writer' src/ docs/`
  - `grep -rn 'tsbpdDelayMs' src/`
- `cargo fmt --all --check` / `cargo check --workspace` / `cargo check --workspace --no-default-features` / `cargo clippy --workspace --all-targets -- --deny warnings` / `cargo clippy --workspace --no-default-features -- --deny warnings` / `cargo test --workspace` がすべて通ること
- `cargo doc --no-deps` の警告数が着手時ベースラインを超えないこと

## CHANGES.md について

`CHANGES.md` には **追記しない**。

- `docs/obsws/PROTOCOL_STATUS.md` の編集分は、shiguredo-changelog 規約「`.rst` / `.md` ファイルの変更は変更履歴に反映しないこと (コード変更と同時に行った場合も、ドキュメント変更分はエントリに含めない)」に従う
- `src/srt/inbound_endpoint.rs` のエラー文言書き換えは利用者影響ゼロの内部命名整合のため、closed 0036 / closed 0022 / closed 0041 と同じ「機能・互換性に影響しない変更は CHANGES.md に記載しない」先例に倣う

## 関連

- closed PR #207 (`feature/remove-json-rpc` / merge commit `d4170ed8`): 本 issue の残骸が生じた契機 (`MediaPipelineHandle` から JSON-RPC 依存除去は `d8151946`)
- closed issue 0041 (`feature/refactor-remove-unused-processor-json-impls` / merge commit `42979dae`): 5 processor 構造体の DisplayJson / TryFrom 削除。本 issue は同 issue のレビューで後追い起票推奨と判定された 2 件を統合
