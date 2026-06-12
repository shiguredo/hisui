# コメント内の英単語混在表記を日本語に統一する

- Priority: Low
- Created: 2026-06-11
- Completed:
- Model: Opus 4.7
- Branch: feature/refactor-japanize-comment-terms
- Polished:

## 目的

ソースコード内のコメント・docstring に英単語と日本語が混在する箇所が多数あり、表記揺れが発生している。プログラム要素・複合用語・慣用表現に該当しない「概念を表す英単語」を日本語化して、コメント表記を一貫させる。

CLAUDE.md には「常に日本語を利用すること」「コメントは全て日本語にすること」と明記されており、本来コメントは日本語で書く方針だが、現状はその方針が徹底されていない。

## 優先度根拠

- 機能影響はゼロ。純粋にコメントの可読性・一貫性向上のための統一作業
- 緊急性は低いが、放置すると新規コメントで同じ表記揺れが繰り返される
- 「直すべき」と分かっている表記揺れが既に複数件存在し、後送りにするほどコストが累積する
- ただしビジネスインパクトや障害発生リスクはないため Medium ではなく Low

## 現状

代表的な表記揺れ:

| 表現 | 種別 | 出現例 |
|---|---|---|
| `codec string` | 概念表現 | `src/codec_string.rs:14`「ビデオとオーディオの SampleEntry から正確な codec string を生成する。」、`src/dash/writer.rs`、`src/hls/writer.rs`、`src/obsws/coordinator/output_*.rs` ほか 40 箇所超 |
| `sample entry` | 概念表現 | `src/sample_entry.rs:1`「映像・音声で共有する sample entry の共通型。」、`src/sample_entry.rs:38`、`src/rtsp/subscriber.rs`、`src/srt/inbound_endpoint.rs` ほか 30 箇所超 |
| `best-effort` | 概念表現 | `src/hls/writer.rs:213`「ファイルを削除する（best-effort、エラーは warning のみ）」 |
| `bind` / `exit` / `return` | 概念表現の動詞 | `src/obsws/server.rs` ほか |
| `finalize 済み` / `metrics 行` | 動詞・名詞混在 | `src/mp4/hybrid_writer.rs`、`src/sora/recording_subcommand_tune.rs` |

`src/codec_string.rs` を例にとると、同一ファイル内で行 1（`コーデック文字列`）と行 14（`codec string`）が混在しており、ファイル内ですら表記が揃っていない。これは feature ブランチ `feature/refactor-encoded-frame-sample-entry-invariant` の作業中に判明した。

## 設計方針

### 対象

- ソースコメント（`//`, `///`, `//!`）と docstring 内の概念表現の英単語

### 対象外

- プログラム要素を参照する英単語
  - 型名（`SampleEntry`, `BrokenPipe`, `Stats`, `MediaPipeline` 等）
  - 関数名・メソッド名・フィールド名・変数名（`pending_video_frame`, `sample_entry`, `.clone()` 等）
  - モジュール名（`decoder`, `writer`, `codec_string` 等）
  - フラグ名・環境変数名・パス（`--emit-startup-info`, `HISUI_EMIT_EXIT_METRICS` 等）
- 技術用語の複合語・固有名詞
  - MP4 ボックス名（`moov`, `mdat`, `moof`）
  - 複合用語（`recovery moov` のようにコードベース内で一連の用語として定着しているもの）
  - プロトコル名・フォーマット名（`HLS`, `DASH`, `RTSP`, `JSON Lines` 等）
- 慣用表現
  - `NOTE:` などの技術文書で慣用化されたマーカー
- 既に日本語として広く流通したカタカナ用語の英語表記
  - `stdout` / `EOF` などの一般化した略語

### ログメッセージ

CLAUDE.md の「ログメッセージは全て英語にすること」規約に従い、ログメッセージは対象外。`tracing::warn!` / `eprintln!` 等の引数は触らない。

### 進め方

- 対象表現ごとに「日本語表記の正解」を決め、その上で grep して文脈確認しながら置換する
- 一括置換ではなく、ファイル単位で文脈を見て判断する（プログラム要素参照と概念表現が同一表記の場合があるため）
- 機能変更や追加リファクタリングは混ぜない

## 完了条件

- 対象表現がソースツリーから消滅する（プログラム要素参照を除く）
- `cargo fmt` / `cargo clippy` / `cargo test` が通る
- 既存のテストが落ちない（コメント変更のみのため挙動変化はない想定）

## 解決方法

1. 表記決定: 主要な表現について日本語表記を確定する
   - `codec string` → `コーデック文字列`
   - `sample entry` → `サンプルエントリー`
   - `best-effort` → `ベストエフォート`
   - `bind` → `バインド`
   - `exit する` → `終了する`
   - `return し` → `戻り`
   - `finalize 済み` → `ファイナライズ済み`
   - `metrics 行` → `メトリクス行`
2. 表現ごとに grep して全件洗い出し、文脈確認しながらコメント部分のみ置換する
3. ログメッセージ・型名・関数名・変数名・モジュール名・フラグ名は変更しない
4. `cargo fmt && cargo clippy && cargo test` で検証
