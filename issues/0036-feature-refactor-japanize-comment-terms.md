# コメント内の英単語混在表記を日本語に統一する

- Priority: Low
- Created: 2026-06-11
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-japanize-comment-terms
- Polished: 2026-06-12

## 目的

CLAUDE.md の「コメントは全て日本語にすること」に従い、`src/` 配下の Rust コメント (`//` / `///` / `//!`) に残る英単語混在表記を、本 issue で確定する 8 表現に限定して日本語表記へ統一する。同時に、既に日本語化された箇所に残るカタカナ表記揺れ (`サンプルエントリ` と `サンプルエントリー` の長音差) も解消する。放置すると新規コメントで同じ揺れが繰り返されるため早期に止める。

文字列リテラル全般 (ログメッセージ、エラーメッセージ、`panic!` / `assert!` / `expect` の引数等) と、コード内の識別子 (型名・関数名・モジュール名・フィールド名・フラグ名・環境変数名) は本 issue の対象外。機能影響はゼロ・コメント文字列の表記統一のみ。

## 現状

`src/` 配下のコメントに以下の英単語混在表記が残っている。件数は `rg -n '^\s*(//|///|//!).*<表現>' --type rust src/` の出力で各表現を縛った実測値 (`#[cfg(test)] mod tests` 内も含む)。

| 対象表現 | コメント内出現数 | 主な出現ファイル |
|---|---|---|
| `codec string` | 44 | `src/codec_string.rs`、`src/dash/writer.rs`、`src/hls/writer.rs`、`src/obsws/coordinator/output_dash.rs`、`src/obsws/coordinator/output_hls.rs` ほか |
| `sample entry` | 6 | `src/sample_entry.rs`、`src/video/h265.rs` |
| `best-effort` / `best effort` | 5 | `src/hls/writer.rs`、`src/metrics.rs`、`src/encoder.rs`、`src/media_pipeline.rs`、`src/mp4/hybrid_writer.rs` |
| `bind` (動詞・概念) | 3 | `src/obsws/server.rs` |
| `exit` (動詞・概念) | 2 | `src/metrics.rs`、`src/obsws/server.rs` |
| `return` (動詞・概念) | 4 | `src/metrics.rs`、`src/obsws/message.rs` (同一文 2 箇所)、`src/tune/nsga2.rs` |
| `finalize` (動詞・状態・時点) | 45 | `src/mp4/writer.rs`、`src/mp4/hybrid_writer.rs`、`src/dash/writer.rs`、`src/hls/writer.rs`、`src/obsws/coordinator/output_dash.rs`、`src/obsws/coordinator/output_hls.rs`、`src/obsws/coordinator/output_record.rs` ほか |
| `metrics 行` | 1 | `src/sora/recording_subcommand_tune.rs` |

`finalize` の `/ cleanup` 並列 (11 件: `finalize / cleanup` 9 件 + `finalize してから cleanup` 2 件) では同時に `cleanup` も `クリーンアップ` に揃える。既存日本語前例: `クリーンアップ` がコメント 4 件 (`src/obsws/state_file.rs` ほか)。本 issue で `cleanup` を統一する対象は `finalize` と並列する 11 件に限定し、それ以外の `cleanup` 単独出現 (現状ゼロ件) を含めない。合計約 121 件。

以下の grep ヒットは識別子参照のため対象外:

- `src/mp4/hybrid_writer.rs:548` の `finalize()` (メソッド呼び出しの参照、ルール 6)
- `src/tune/nsga2.rs:375` の `finalize の clamp` (同ファイル 388 行で `range.finalize(child)` を呼ぶ `NumericRange::finalize` メソッドの動作説明。ルール 8 単独適用なら概念表現になり得るが、ルール 9 で識別子側に倒す)
- `src/subcommand_server.rs:219` の `return が必要` (直下の `#[expect(clippy::needless_return)]` 属性が示すとおり Rust の `return` キーワード自体への言及。ルール 9 で識別子側に倒す)

カタカナ表記揺れ:

| 既存表記 | 件数 | 統一先 |
|---|---|---|
| `サンプルエントリ` (長音なし) | 4 (`src/video.rs:293, 433`、`src/decoder/nvcodec.rs:96, 228`) | `サンプルエントリー` (33 件・多数派) に統一 |

## 設計方針

### スコープ

- 対象: `src/` 配下の Rust ファイル内のコメント (`//` / `///` / `//!`)。`#[cfg(test)] mod tests` 内も対象 (CLAUDE.md「テストはコメントを重視すること」を適用)。
- 対象外: 文字列リテラル全般 (ログマクロ引数、`crate::Error::new("...")` / `format!("...")` のエラー文字列、`panic!` / `assert!` / `expect` 等の文字列リテラル)。CLAUDE.md「ログメッセージは全て英語にすること」と整合させるため触らない。
- 対象外: コード内の識別子 (型名・関数名・メソッド名・フィールド名・変数名・モジュール名・パス・フラグ名・環境変数名)。
- 既出 8 表現と `/ cleanup` 連動分、および 1 カタカナ揺れに限定する。他の英単語混在 (例: `flush`, `pending`, `channel`, `attach` 等) は本 issue の対象外。

### 表記決定

| 対象表現 | 統一先 | 既存日本語前例 |
|---|---|---|
| `codec string` | `コーデック文字列` | `src/codec_string.rs` 他にコメント 6 件 |
| `sample entry` | `サンプルエントリー` (長音あり) | `src/encoder/openh264.rs` 他にコメント 33 件 |
| `best-effort` / `best effort` | `ベストエフォート` | `src/dash/writer.rs`、`src/mp4/demuxer.rs` にコメント 3 件 |
| `bind` | `バインド` | (新規) |
| `exit` する | `終了する` | `src/encoder.rs:376`、`src/media_pipeline.rs:96` ほか頻出 |
| `return` する/し | `関数を抜ける` (動詞) / `早期復帰` (名詞句限定。下記の確定置換参照) | (新規) |
| `finalize` (動詞・状態・時点) | `ファイナライズ` 系で機械置換 (下記補足参照) | (新規) |
| `cleanup` (`finalize` 並列分のみ) | `クリーンアップ` | コメント 4 件 (`src/obsws/state_file.rs` 他) |
| `metrics 行` | `メトリクス行` | `src/sora/recording_subcommand_tune.rs:305` (`終了時メトリクス行`) |
| `サンプルエントリ` | `サンプルエントリー` | (多数派) |

#### `return` の確定置換

該当 4 件は全て文脈確定済み。判定ルールを経ずに以下の通り個別置換する。

| 該当箇所 | before | after |
|---|---|---|
| `src/metrics.rs:12` | `警告ログのみで return し、終了処理は妨げない` | `警告ログのみで関数を抜け、終了処理は妨げない` |
| `src/obsws/message.rs:1557, 1599` | `その場合はコード値だけ確認して早期に return する。` | `その場合はコード値だけ確認して早期に関数を抜ける。` |
| `src/tune/nsga2.rs:499` | `全個体がそのまま親になる (早期 return 経路)` | `全個体がそのまま親になる (早期復帰経路)` |

#### `finalize` の置換補足

事前確認済みの 45 件 (47 件 grep ヒット - 識別子参照 2 件) はルール 8 で全て概念表現扱いと判定済みのため、個別判断不要で `ファイナライズ` に機械置換できる。意訳は不要。

- 接尾辞・修飾語 (`時 / 後 / 中 / 済み / 未完了 / 完了前 / 失敗 / 成否 / 経由 / まで / 固有 / 直前` 等)、動詞活用 (`を促す` / `して(から)` / `を経ずに` / `を経由` / `を優先` / `に進む` / `へ遷移` 等)、全角括弧での補足 (`finalize（標準 MP4 への変換）`)、矢印遷移 (`→ finalize`) は全て `ファイナライズ` で機械置換できる。
- `/ cleanup` と並列される箇所では `cleanup` も同時に `クリーンアップ` に揃える (`ファイナライズ / クリーンアップに進ませる` 等)。該当 11 件は `rg -n '^\s*(//|///|//!).*\bcleanup\b' --type rust src/` で確認できる。

### プログラム要素と概念表現の判定ルール

同じ綴りがプログラム要素 (識別子) としても概念表現としても出現する。次のルールを上から順に適用する。

1. **バッククォートで囲まれている場合は識別子扱い** (対象外): `` `finalize()` ``, `` `SampleEntry` `` など
2. **Markdown の intra-doc link 記法 `[Foo]` で囲まれている場合は識別子扱い** (対象外): `[SampleEntry]`, `[NOTE]` など
3. **`::` を含む Rust パス記法は識別子扱い** (対象外): `std::process::exit`, `NumericRange::finalize` など (バッククォート無しでも)
4. **アンダースコアを含む snake_case 表記は基本的に識別子扱い** (対象外): `pending_video_frame`, `sample_entry` など
   - 例外: 周辺コード (同関数・同 impl・同モジュール) に対応する識別子 (フィールド・変数・関数) が存在しない場合は、英語句を snake_case 風に表記した概念表現とみなし対象に含める。
   - 例: `/// 最後に受信したビデオの sample_entry（SPS/PPS 注入...` → 周辺コードに `sample_entry` フィールド/変数が無ければ `/// 最後に受信したビデオのサンプルエントリー（SPS/PPS 注入...`
5. **PascalCase または 2 文字以上の連続した大文字 (数字を含んでもよい) で構成される略号は識別子扱い** (対象外): `SampleEntry`, `CodecString`, `MP4`, `H265`, `HLS`, `DASH`, `RTSP`, `JSON`, `AV1`, `AAC`, `EOS` など (型名・プロトコル名・フォーマット名・コーデック名)
6. **メソッド呼び出し記法 `.foo()` / `foo()` は識別子扱い** (対象外): `.finalize()`, `bind()` など
7. **`--flag` / `HISUI_xxx` のフラグ名・環境変数名は識別子扱い** (対象外)
8. **それ以外で「日本語の助詞 (`を`, `が`, `は`, `に`, `の` 等) または接尾辞・修飾語 (`完了`, `後`, `時`, `中`, `済み`, `直後`, `失敗`, `未完了`, `等` 等) が前後に接続される」場合は概念表現扱い** (対象)
   - 例: `bind 完了直後` → `バインド完了直後`
   - 例: `bind 等の .await より前` → `バインド等の .await より前`
   - 例: `best-effort 出力` → `ベストエフォート出力`
   - 例: `（best effort）` → `（ベストエフォート）`
9. **判断に迷う境界は識別子側 (保守側) に倒す**。誤って識別子参照を日本語化するより、概念表現が少し残る方が安全。`src/tune/nsga2.rs:375` (`finalize の clamp`) と `src/subcommand_server.rs:219` (`return が必要`) はルール 8 単独適用なら概念表現になり得るが、いずれも近接する識別子参照を意図しているためルール 9 で対象外とする (本 issue では現状セクションで先回り除外済み)。

`src/video/h265.rs:19, 82` のように関数 doc コメントで戻り値型 `SampleEntry` を意図する `sample entry` も、ルール 8 を適用して `サンプルエントリー` に統一する。

### 進め方

`finalize` (45 件 + `cleanup` 連動 11 件) は単独 PR とし、他表現とは分ける (レビュー負荷の観点)。それ以外は 1 PR にまとめても、表現ごとに PR を分けてもよい。

## 完了条件

- 以下の `rg` 検証で対象表現が概念表現として残っていないこと。`bind` / `exit` / `return` / `finalize` は識別子としても出現するため、grep 結果を目視して残存箇所がルール 1〜7 の識別子扱い、または現状セクションで先回り除外した 3 箇所 (`src/mp4/hybrid_writer.rs:548`、`src/tune/nsga2.rs:375`、`src/subcommand_server.rs:219`) に限ることを確認する (ゼロ件には到達しない)。

  ```sh
  # 機械的に「ゼロ件」が達成可能なもの
  rg -n '^\s*(//|///|//!).*(codec string|sample entry|best[- ]effort|metrics 行)' --type rust src/

  # 目視で識別子扱いのみが残ることを確認するもの
  rg -n '^\s*(//|///|//!).*\b(bind|exit|return|finalize)\b' --type rust src/
  ```

- `サンプルエントリ` (長音なし) と、`finalize` に並列する英語の `cleanup` の出現がゼロ件であること:

  ```sh
  rg -nP 'サンプルエントリ(?!ー)' --type rust src/
  # 英語の `finalize` と `cleanup` が同一コメント行に共起する箇所がないこと (両方カタカナ化済みなら 0 件)
  rg -n '^\s*(//|///|//!).*\b(finalize.*cleanup|cleanup.*finalize)\b' --type rust src/
  ```

- `cargo fmt --check` / `cargo clippy` / `cargo test` が通り、`cargo doc --no-deps` の警告数が本 issue 着手前のベースラインを超えないこと。ベースラインは着手時点で develop ブランチ上で `cargo doc --no-deps 2>&1 | grep -E '^warning:' | wc -l` を実行して再計測する。

## 再発防止

本 issue は close 後に同種の英単語混在表記が新規追加された際の前例として参照される想定。発生時は以下に従って都度修正する。

- 「表記決定」表の訳語を最優先で適用する。表に無い新規対象表現は別 issue で扱う (本 issue を再オープンしない)。
- 「プログラム要素と概念表現の判定ルール」(9 条) は対象表現を問わず汎用的に適用できる。新規表現にもそのまま使ってよい。
  - 特にルール 4 (snake_case の例外: 周辺コードに対応識別子が無ければ概念表現扱い) とルール 5 (PascalCase / 全大文字略号は識別子扱い) は、英語句を snake_case 風に書くケースと、フォーマット名 (`MP4`, `HLS` 等) を残すケースを区別するのに有用。
- 「完了条件」の `rg` コマンドは監査スクリプトとしてそのまま再利用できる。発生疑いがあれば単発実行して確認する。

CI 化 (`rg` ベースの自動チェック) は本 issue のスコープ外。発生頻度が無視できなくなった場合は別 issue で検討する。

## CHANGES.md について

機能・互換性に影響しないコメント変更のため CHANGES.md には記載しない。先例: `feature/refactor-fmp4-reader-naming` (closed 0022) でも同様の判断を取っている。

## 関連

- `0030 feature/refactor-encoded-frame-sample-entry-invariant`: 対象ファイルが重なるため、本 issue 着手時点で develop の最新を取り込んでから作業すること。特に `src/sample_entry.rs`、`src/encoder/openh264.rs`、`src/mp4/hybrid_writer.rs` で重複の可能性がある。
