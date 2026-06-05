# inspect の format で fMP4 を "fmp4" として返す

- Priority: Low
- Created: 2026-06-04
- Completed: 2026-06-05
- Model: Opus 4.8
- Branch: feature/add-inspect-fmp4-format-distinction
- Polished: 2026-06-04

## 目的

inspect は fMP4 入力でも `format` フィールドを `"mp4"` として返し、通常 MP4 と区別しない。fMP4 を判別できるよう、fMP4 のとき `format` を `"fmp4"` として返す（通常 MP4 は `"mp4"`、WebM は `"webm"` のまま）。

## 優先度根拠

Low。判別できなくても実害は小さい。

## 後方互換

- 変えるのは fMP4 のときの `format` だけ。通常 MP4（`"mp4"`）・WebM（`"webm"`）の出力は不変。
- fMP4 inspect 機能（issue 0001）は既に develop にマージ済みだが、それを含む安定版はまだリリースされていない（現行バージョンは `2026.1.0-canary.2`）。安定版リリース前に `"fmp4"` へ変えれば、互換破壊ではなく新機能の出力仕様を確定するだけになる（CHANGES 上も fMP4 対応 [ADD] の一部）。
- canary には既に fMP4 に対する `format: "mp4"` が乗っている。安定版リリース前に変えれば安定版利用者には `"mp4"` を一度も出さずに済む（canary を pre-release として非保証扱いにできるかは要確認）。いずれにせよリリース後に変えると [CHANGE] になるため、着手するなら早い方がよい。

## 現状

- `src/types.rs` の `ContainerFormat` は `Mp4` / `Webm` の 2 値のみ（`Fmp4` なし）。`ContainerFormat::from_path` は拡張子だけで判定（`.mp4` → `Mp4`、`.webm` → `Webm`、それ以外は Err）し、ファイル実体（ftyp / moov）は見ない。
- inspect の `format` は `OutputPrinter` が保持する `ContainerFormat` を `DisplayJson`（`Mp4 => "mp4"`）で出力する。`--decode` の有無に関わらず fMP4 入力（`.mp4`）でも `"mp4"`。
- issue 0001 は意図的に「段階 1 では mp4 と fmp4 を区別しない（`"format": "mp4"`）」かつ「`ContainerFormat` は webm/mp4 据え置き、`Fmp4` は追加しない（外部 API 互換維持）」と先送りした。本 issue はこれを覆して `Fmp4` を追加する（外部 API 互換は下記の `TryFrom` 据え置きで維持する）。

## 設計方針

`ContainerFormat` に `Fmp4` を追加する。ただし content 判定は `from_path` には入れず、inspect 層の別関数に切り出す。

### src/types.rs

- `ContainerFormat` に `Fmp4` を追加する（`DisplayJson` の arm は後述の「網羅性 arm」一覧参照）。
- `from_path` は変更しない（拡張子判定のまま、`.mp4` は `Mp4`）。`detect_mp4_file_kind` を `from_path` 内で呼ばない（基盤モジュール `types` が `mp4` に依存する逆方向の層依存を避けるため）。`.fmp4` のような拡張子は従来どおり `from_path` が Err で弾く（fMP4 は `.mp4` で配布されるのが通常という 0001 の前提を踏襲）。fMP4 判定は `.mp4` ファイルの中身に対してのみ行う。
- `TryFrom<RawJsonValue>` は据え置く（`"webm"` / `"mp4"` のみ受理、それ以外は Err）。これにより Sora 録画メタデータのパース（`src/sora/recording_metadata.rs` が `ContainerFormat::try_from` 経由で構築する）は `Fmp4` を生成せず、Sora ドメインを汚さない（= 0001 が守ろうとした外部 API 互換の維持）。

### src/subcommand_inspect.rs

- content 判定関数 `detect_container_format(path: &Path) -> crate::Result<ContainerFormat>` を新設し、`run_internal` の `ContainerFormat::from_path(&input_file_path)?` を置換する。`from_path` の結果が `Mp4` のとき `detect_mp4_file_kind`（`src/mp4/file_kind.rs`、`pub(crate)`、戻り値 `Result<Mp4FileKind>`）を見て `Mp4FileKind::FragmentedMp4` なら `Fmp4` に補正する。
- `detect_mp4_file_kind` がエラー（破損ファイル等）のときはエラーを伝播して inspect を失敗させる（`Mp4` へフォールバックしない）。後段の reader 初期化（`Mp4Demuxer::open`）でも同じ判定で失敗するため情報は失われず、堅牢性優先（CLAUDE.md）に沿う。
- reader 選択 match は fMP4 も同じ前方読み reader を使う（`Mp4 | Fmp4 => Mp4SampleReader`。arm 本体は既存の `Mp4` ブロックと共通でよい）。

### Fmp4 追加で網羅性 arm が必要な match（全箇所）

`Fmp4` を追加すると、`ContainerFormat` を分岐している以下の exhaustive match すべてに arm が要る（grep `ContainerFormat::` で確認できる。`TryFrom` は `&str` を match し catch-all を持つので対象外）。各 arm の戻り値も示す:

- `src/types.rs` の `DisplayJson`: `Fmp4 => f.string("fmp4")`（既存の `f.string` 方式に揃える）
- `src/subcommand_inspect.rs` の reader 選択（`run_internal` 内）: `Mp4 | Fmp4 => Mp4SampleReader`
- `src/sora/recording_reader.rs` の `AudioReader::new` / `VideoReader::new` の match: `Mp4 | Fmp4 => <Mp4 と同じ reader>`
- `src/sora/recording_subcommand_compose.rs` の audio / video reader 選択: `Mp4 | Fmp4 => "mp4_audio_reader" / "mp4_video_reader"`
- `src/sora/recording_layout.rs` の `set_extension` 分岐: `Mp4 | Fmp4 => set_extension("mp4")`

Sora 系（`recording_*`）の match では `Fmp4` は到達しない（`TryFrom` が生成しないため）。ここは exhaustive 維持のための機械的な補完であり、録画合成の実 fMP4 対応（0001 の段階 2a）は独立 issue 化されておらず、issue 0016 で「ついで対応」とされている（「関連 issue」参照）。

## テスト

- `tests/e2e.rs`:
  - `inspect_fragmented_mp4_video_only`（現状 `format == "mp4"` を assert）: 期待値を `"fmp4"` に修正する。
  - `inspect_fragmented_mp4_audio_only` / `inspect_fragmented_mp4_audio_video`（現状 `format` の assert なし）: `format == "fmp4"` の assert を追加する。
  - `inspect_mp4_*`（通常 MP4）と `inspect_webm_*` は `"mp4"` / `"webm"` のまま不変であることを維持する（回帰検出）。
- `detect_container_format` の単体テスト: 正常系（`.mp4` 通常 → `Mp4`、`.mp4` fragmented → `Fmp4`、`.webm` → `Webm`）と破損ファイルのエラー伝播を検証する。`src/subcommand_inspect.rs` に新規の `#[cfg(test)] mod tests` を追加する（既存の `src/mp4/file_kind.rs` の in-src テスト `detect_regular_mp4` 等が手本。なお本リポジトリは `pbt/` を持たず `tests/` は `<module>_tests.rs` 命名で、CLAUDE.md の `tests/test_<module>.rs` 規約とは実態が乖離している。ここは in-src テストの先例に合わせる）。
- Python e2e（`e2e-tests/obsws/test_output.py`）: `inspect_output["format"] == "mp4"` のアサーションが複数あるが、いずれも Sora 録画の通常 MP4 出力を inspect しているため `"mp4"` のままで影響しない見込み。実装時に、fMP4（HLS / MPEG-DASH の fMP4 セグメント等）を `hisui inspect` して `format` を assert している箇所が無いことを確認する。

## CHANGES.md

- `## develop` の既存エントリ `[ADD] inspect コマンドが fMP4 ファイルの読み込みに対応する` に、sub-bullet として「inspect は fMP4 を `format: "fmp4"` として返す」を追記する（既存の複数 sub-bullet エントリと同じ書き方）。担当者行（`- @ユーザー名`）は本対応の実装者のものを足す。

## 関連 issue

- issue 0020（inspect fMP4 の e2e を通常 MP4 と突き合わせる）: 同じ `inspect_fragmented_*` テストを改修対象とするため、作業順序・統合を調整する。
- issue 0021（`Mp4Demuxer::open` の二度読み解消）: `detect_container_format` が `detect_mp4_file_kind` を呼ぶことで inspect の判定回数が増える。現状 inspect の `Mp4SampleReader` は `Mp4Demuxer::open` を 2 回呼び、`detect_mp4_file_kind` が 2 回走っている。本対応で 3 回になる。本 issue では許容するか、0021 で判定結果を後段に引き回す設計にしてから本 issue を載せるかを決める。
- issue 0022（fMP4 reader の命名・エラー文言整理）: 関数名 `detect_container_format` が 0022 の命名整理と非整合にならないよう調整する。
- issue 0016（OBSWS メディア再生 = 0001 段階 2b の fMP4 対応）: Sora 系 match に足す `Fmp4` arm は exhaustive 維持のための補完。録画合成（段階 2a）の実 fMP4 対応は独立 issue が無く、0016 で「ついで対応」とされている。

## 実装ブランチ

- fMP4 inspect 機能は既に develop にマージ済み（旧 `feature/add-fmp4-read-support` は develop に取り込み済み）なので、develop から新規に `feature/add-` 系ブランチを切って実装する。後方互換を壊さない出力仕様確定なので prefix は `feature/add-`。

## 完了条件

- fMP4 を inspect したとき `format` が `"fmp4"` になり、通常 MP4 は `"mp4"`、WebM は `"webm"` のままであること。
- `ContainerFormat::Fmp4` が Sora 録画メタデータのパース経路では生成されないこと。
- 上記テスト（Rust e2e の修正・追加、`detect_container_format` の単体テスト）が通り、Python e2e に回帰がないこと。
- CHANGES.md に反映すること。
- 安定版リリース前にマージすること（リリース後だと [CHANGE] になる）。

## 解決方法

設計方針どおり実装した。

- `src/types.rs`: `ContainerFormat` に `Fmp4` を追加。`DisplayJson` に `Fmp4 => f.string("fmp4")` を追加。`from_path`（拡張子判定）と `TryFrom<RawJsonValue>`（`"webm"` / `"mp4"` のみ受理）は据え置き、Sora 録画メタデータ経路では `Fmp4` を生成しないようにした。
- `src/subcommand_inspect.rs`: content 判定関数 `detect_container_format` を新設し、`run_internal` の `ContainerFormat::from_path` を置換。`Mp4` のとき `detect_mp4_file_kind` を見て `FragmentedMp4` なら `Fmp4` に補正する。判定エラーはそのまま伝播し `Mp4` へフォールバックしない。reader 選択 match は `Mp4 | Fmp4` で共通化。
- `src/sora/recording_reader.rs`・`src/sora/recording_layout.rs`・`src/sora/recording_subcommand_compose.rs` の `ContainerFormat` match に `Fmp4` arm を追加。これらの Sora 録画メタデータ経路は `ContainerFormat::try_from`（`"webm"`/`"mp4"` のみ受理）経由でしか `format` を構築せず `Fmp4` は生成されないため、`Mp4` と同じ扱いで握りつぶさず、到達した場合は実装バグとして明示的にエラー（`unexpected fMP4 container format in Sora recording metadata`）を返すようにした。
- テスト: `tests/e2e.rs` の `inspect_fragmented_mp4_video_only` を `format == "fmp4"` に修正、`inspect_fragmented_mp4_audio_only` / `inspect_fragmented_mp4_audio_video` に `format == "fmp4"` の assert を追加。`detect_container_format` の単体テスト（通常 MP4 / fMP4 / WebM / 破損 MP4 のエラー伝播）を `src/subcommand_inspect.rs` に追加。
- `CHANGES.md`: 既存の fMP4 対応 [ADD] エントリに sub-bullet を追記。

全テスト（575 + e2e）と clippy がパスすることを確認した。
