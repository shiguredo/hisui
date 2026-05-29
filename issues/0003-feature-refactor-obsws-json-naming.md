# obsws / obsdc 系の JSON フィールドの命名規則を統一する

- Priority: Medium
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-obsws-json-naming
- Polished: 2026-05-29

## 目的

hisui の OBS WebSocket 互換サーバー（`src/obsws/` 配下）と、WebRTC DataChannel 経由で同種のメッセージを流す obsdc（DataChannel label `obsdc`）系のリクエスト・レスポンス JSON で、フィールド名の命名規則 (`camelCase` / `snake_case`) が箇所によってばらついている。OBS Studio 本体側でも命名規則が揺れており、その揺れがそのまま hisui に持ち込まれている形だが、**hisui 内で独自に定義しているフィールドについては命名規則を統一**したい。本対応で、どのフィールドが「OBS 互換のために触れない」/「hisui の責務で統一すべき」かを切り分け、後者を camelCase に揃える。

## 優先度根拠

- 命名規則のばらつきは外部 API のスペック揺れに直結し、UI 側 (devtools)・SDK 利用者・E2E テスト作者すべてに認知負荷を強いる。
- 一方、ばらつきがあっても現状の機能は動くため緊急度は High ではない。
- リネームは後方互換のない変更を含むため、リリースタイミング・移行コストを考えると Medium 妥当。
- 早めに方針を固定しないと、新規エンドポイント追加時に「どっちに合わせるべきか」の判断が都度発生してコストが累積するため、先送りはしない。

## 現状

### OBS WebSocket 標準フィールド（揺れていない、変更しない部分）

`src/obsws/response/`, `src/obsws/state/types.rs` などのほとんどは OBS WebSocket Protocol v5 の定義に従って camelCase で書かれている。例:

- `src/obsws/response/general.rs:109-126`: `obsVersion`, `obsWebSocketVersion`, `rpcVersion`, `supportedImageFormats`, `availableRequests` 等
- `src/obsws/response/input.rs:324, 429, 568-569`: `inputUuid`, `sceneItemId`, `inputMuted`, `inputVolumeDb`, `inputVolumeMul`
- `src/obsws/state/types.rs:75-81`: `inputName`, `inputKind`, `inputUuid`, `inputKindCaps`
- `src/obsws/state/types.rs:576-594`: scene item transform の `positionX`, `cropTop`, `boundsType` 等

これらは **OBS WebSocket Protocol 仕様で文字列が確定している** ため、命名規則統一の対象外。

### OBS WebSocket 仕様内で大文字スネークが使われる箇所（標準準拠なので変更しない）

- `src/obsws/response/general.rs:295-299`: capability flag の `MAIN`, `ACTIVATE`, `MIX_AUDIO`, `SCENE_REF`, `EPHEMERAL`
- `src/obsws/protocol.rs` などの subscription / action 定数 (`OBSWS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY` 等)

これらは OBS Studio 仕様で大文字スネークが正なので対象外。

### OBS Studio 本体の Source 設定で snake_case が使われる箇所（互換のため変更しないものを切り分け要）

OBS Studio 本体で source plugin (`video_capture_device`, `audio_capture_device`, stream service など) が `obs-data` 上で snake_case のキーを使っているため、その input settings をリクエスト・レスポンス JSON で受け渡しする部分も snake_case になっている。

- `src/obsws/state/types.rs:995, 998`: `ObswsVideoCaptureDeviceSettings` の `device_id`, `pixel_format`
- `src/obsws/state/types.rs:1021`: `ObswsAudioCaptureDeviceSettings` の `device_id`
- `src/obsws/coordinator/output_registry.rs:478`: `GetStreamServiceSettings` の `use_auth`
- `src/obsws/message.rs:1131`: 受信側で `to_member("device_id")` してパース

これらは OBS Studio 互換を保つために **そのまま snake_case を維持** すべき。

### Sora シグナリング由来で snake_case の箇所（外部仕様準拠なので変更しない）

- `src/obsws/coordinator/output_sora.rs:522-550`: Sora シグナリングメッセージの `event_type`, `connection_id`, `client_id`

これは Sora の仕様準拠なので対象外。

### hisui 独自フィールドのばらつき（統一の対象）

最大の問題は **同一構造体の中で camelCase と snake_case が混在している** 箇所がある点。

- `src/obsws/state/types.rs:1017-1029` (`ObswsAudioCaptureDeviceSettings`)
  - `device_id` (snake_case、OBS 互換のため維持)
  - `sampleRate` (camelCase)
  - `channels` (区別なし)
  - → `device_id` と `sampleRate` が同居しており、ここが OBS 互換派由来か hisui 独自か判別が一見してできない。
- `src/obsws/state/types.rs:1000-1001` (`ObswsVideoCaptureDeviceSettings`)
  - `device_id` / `pixel_format` (snake_case) と `fps` (区別なし) が同居。`fps` は両規則どちらでも同じ綴りになるので問題は表面化しないが、判断基準が不明瞭。
- hisui 独自の input kind では camelCase で揃っている (`loopPlayback`, `inputUrl`, `streamName`, `streamId`, `trackId`, `backgroundKeyColor`, `videoTrackId` 等。`src/obsws/state/types.rs:1058-1170`)。これは方針として **camelCase に決め切る** のが妥当な根拠になる。

### obsdc 系の扱い

- `src/webrtc/p2p_session.rs:612-627` で `obsdc` DataChannel を作成し、obsws と同種の OBS WebSocket メッセージを WebRTC DataChannel で送受信する。
- メッセージ本体は obsws と共有 (`src/obsws/message.rs`) しているため、JSON 命名規則は obsws と共通で扱う。本 issue で決めた規約は obsdc にも自動的に適用される。
- 「obsws / obsdc」と並列に書いているが、**実装上は同じ JSON コードパスを通る**ため、追加の対応はない（規約適用範囲を明文化するのみ）。

## 設計方針

### 規約

以下の優先順位で個々のフィールド名を決める。

1. **OBS WebSocket Protocol v5 で命名が確定しているフィールド**: 仕様準拠 (大半は camelCase, 一部 SCREAMING_SNAKE_CASE)。**変更しない**。
2. **OBS Studio 本体の Source 設定として snake_case が事実上の標準になっているフィールド**: OBS Studio クライアントとの相互運用性のため **snake_case を維持**。対象は現状確認できているもののみ列挙し、追加判断は OBS Studio のソース定義 (`obs-studio` リポジトリ) を確認した上で行う。
   - `video_capture_device` の `device_id`, `pixel_format`
   - `audio_capture_device` の `device_id`
   - stream service settings の `use_auth`, `bwtest`
3. **外部プロトコル準拠で snake_case のフィールド**: Sora シグナリングなど。**外部仕様の表記をそのまま維持**。
4. **hisui が独自に定義するフィールド**: **camelCase に統一**。
   - 例: `loopPlayback`, `inputUrl`, `streamName`, `streamId`, `trackId`, `backgroundKeyColor`, `videoTrackId`, `sampleRate`（既存の camelCase 例）
   - 新規追加時もこれに従う。
5. **判断が割れるグレーゾーン**: 例えば `audio_capture_device` の `sample_rate`/`sampleRate`、`video_capture_device` の `fps` などは、OBS Studio の Source 定義に該当の項目があるかどうかで決める。
   - OBS Studio 本体で同名のキーがある → snake_case を維持。
   - OBS Studio 本体に該当キーが無い (= hisui 独自拡張) → camelCase。
   - 判断が付かない場合は issue 内 PR でレビューを受けて決める。

### Sora 由来のフィールドについての例外整理

`output_sora.rs` の `event_type` 等は受信メッセージのパース時にしか登場しない (Sora から来る JSON をそのまま読む箇所)。これは hisui のレスポンス命名規則とは別扱いとし、Sora シグナリング由来のフィールドは Sora 仕様準拠を明文化する。

### 大文字小文字の判定アルゴリズム

レビュー時の判断負荷を下げるため、簡易チェックリストを `docs/` 配下に置く。

```
1. OBS WebSocket 仕様の Requests/Events ページに同名フィールドはあるか？ → あればその表記を採用。
2. OBS Studio の `obs-source-info` または該当 Source 実装に同名キーはあるか？ → あればその表記を採用。
3. 外部プロトコル (Sora 等) から来るデータか？ → 外部仕様の表記を採用。
4. 以上に該当しない hisui 独自フィールドは camelCase。
```

ドキュメント整備は本対応のスコープ外（CLAUDE.md に従い、ドキュメントは別途）だが、判断基準は本 issue 内に書き残す。

## 完了条件

- 上記規約に従い、`src/obsws/` 配下の hisui 独自フィールドが camelCase に統一されていること。
- OBS WebSocket 仕様準拠の部分・OBS Studio Source 互換のために snake_case を維持する部分は変更されていないこと（差分を見れば触っていないことが明らかであること）。
- 同一構造体内に camelCase と snake_case が混在する箇所が、規約上の根拠 (OBS 互換 / hisui 独自) で説明できるようコメントが付与されていること。
- 既存テスト (`cargo test`) がすべて成功すること。テストデータ内の JSON 期待値も合わせて更新すること。
- 後方互換に関する明示:
  - リネーム対象のフィールドは CHANGES.md の `## develop` に `[CHANGE]` として列挙すること。
  - state file (`HISUI_SERVER_STATE_FILE` で読み書きしている永続データ) のフィールド名は **特に慎重に**: 既存 state file が読めなくなる場合は移行コードを用意するか、別 issue として切り出す。
- リネーム後、OBS Studio 本体クライアントを hisui server に繋げて動作確認したログまたは記録を残す（互換性に副作用がないことの確認）。

## 解決方法

### 実装ステップ

1. 既存の hisui 独自フィールドを洗い出すスクリプト/コマンドを `scripts/` 配下に追加する（grep ベースで `f.member("...")` と `to_member("...")` を列挙し、人手で「OBS 標準 / OBS Source / Sora / hisui 独自」のタグ付けを行う）。
   - 例: `rg 'f\.member\("[a-zA-Z_]+"' src/obsws | sort -u`
2. タグ付け結果を本 issue にコメント追記し、camelCase に統一する対象のリストを確定する。
3. リネーム実施。`f.member("...")` の出力側だけでなく、`to_member("...")` の受信側、`tests.rs` 内の期待値、`testdata/` の固定 JSON も同時に更新する。
4. state file のフィールドが対象になった場合は、`src/obsws/state_file.rs` で旧キーをフォールバック読み込みするコード（最低 1 リリースは互換維持）を追加するか、移行ガイドを書いて非互換変更とするかを決める。後者で行く場合は CHANGES.md に明示。
5. devtools (`devtools/`) など UI 側で対象フィールドを直接読み書きしている箇所があれば、同 PR で揃える。
6. CHANGES.md の `## develop` に対象リストを `[CHANGE]` として列挙する。
7. OBS Studio 本体 (公式クライアント) を立ち上げて hisui server に接続し、Source 設定（video_capture_device, audio_capture_device, stream service）が正しく往復することを確認する。

### 留意事項

- `ObswsAudioCaptureDeviceSettings` のように、**同じ input kind の中で snake_case と camelCase が必然的に混在する**ケースがある。これは「OBS 互換のため device_id は触らない、sample_rate は hisui 独自と判断したので sampleRate にする」のように **コメントで根拠を残す**。読み手が「揺れている」と勘違いしないことが重要。
- 受信側 (`message.rs` 等の `to_member`) で旧名と新名の両方を試すフォールバックコードを **入れない**: hisui server は単一バイナリでバージョンごとに切り替わるので、フォールバックを残すと規約が緩む。互換性が必要なら state file 経由でのみ対応する。
- リネームによる git の blame が広範囲に汚染されるが、許容する。コミットを「リネームのみ」に絞り、ロジック変更を別コミットに分けることで読みやすくする。
- 移行コストを抑えたい場合、まず「混在を解消し、独自フィールドだけ統一する」コミットを 1 つ、「規約・コメントを足す」コミットを 1 つ、で分けて段階的にレビューする。
