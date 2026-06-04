# inspect の format で fMP4 を "fmp4" として返す

- Priority: Low
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

現状 inspect は fMP4 入力でも `format` フィールドを `"mp4"` として返し、通常 MP4 と区別しない。fMP4 を判別できるよう、fMP4 のとき `format` を `"fmp4"` として返す。

## 優先度根拠

Low。現状の `"mp4"` 固定でも実害はないが、fMP4 を inspect で判別できると有用。fMP4 機能のリリース前に入れれば後方互換の問題なく対応できる（下記参照）。

## 決定事項

- 案 B を採用する: fMP4 入力のとき inspect の `format` を `"fmp4"` として返す（通常 MP4 は `"mp4"`、WebM は `"webm"` のまま）。
- 後方互換は崩れない: 通常 MP4 / WebM の `format` は不変。fMP4 を inspect できる機能自体が未リリース（`feature/add-fmp4-read-support` の canary のみで develop 未マージ）で、fMP4 に対して `"mp4"` を返す挙動は正式にリリースされていない。fMP4 機能がリリースされる前に `"fmp4"` を返すようにすれば、互換破壊ではなく新機能の出力仕様を確定するだけになる（CHANGES 上も fMP4 対応 [ADD] の一部として扱う）。

## 現状

- `src/types.rs` の `ContainerFormat` は `Mp4` / `Webm` の 2 値のみ（`Fmp4` なし）。`ContainerFormat::from_path` は拡張子だけで判定し、ファイル実体（ftyp / moov）を見ない。
- inspect の `format` フィールドは `ContainerFormat` の JSON 出力（`Mp4 => "mp4"`）で、`--decode` の有無に関わらず fMP4 入力でも `"mp4"`。e2e テスト `inspect_fragmented_mp4_video_only` が `format == "mp4"` を assert 済み。
- これは issue 0001 で「段階 1 では mp4 と fmp4 を区別しない。区別したい要望が出てきたら別途検討する」と明示的に先送りされた項目。

## 設計方針

`ContainerFormat` に `Fmp4` を追加する。ただし content 判定は `from_path` には入れず、inspect 層の別関数に切り出す。

- `src/types.rs`:
  - `ContainerFormat` に `Fmp4` を追加。`DisplayJson` に `Fmp4 => "fmp4"` を追加する。
  - `from_path` は変更しない（拡張子判定のまま。`.mp4` は `Mp4` を返す）。`detect_mp4_file_kind` を `from_path` 内で呼ばない（基盤モジュール `types` が `mp4` に依存する逆方向の層依存を作らないため）。
  - `TryFrom<RawJsonValue>` は据え置き（`"webm"` / `"mp4"` のみ受理）。これにより Sora 録画メタデータのパースは `Fmp4` を生成せず、Sora ドメインを汚さない。
- `src/subcommand_inspect.rs`:
  - content 判定の別関数（仮称 `detect_container_format`）を新設する。`ContainerFormat::from_path` の結果が `Mp4` のとき `detect_mp4_file_kind` を見て `FragmentedMp4` なら `Fmp4` に補正する。inspect はこの関数で `format` を決める。
  - reader 選択の match は `Mp4 | Fmp4 => Mp4SampleReader`（fMP4 も同じ前方読み reader を使う）。
- 他の `ContainerFormat` の match（`src/sora/recording_layout.rs` / `recording_reader.rs` / `recording_subcommand_compose.rs`）は網羅性のため `Mp4 | Fmp4 => <Mp4 と同じ>` の arm を足す。Sora ドメインでは `Fmp4` は到達しない（`TryFrom` が生成しない）が、将来 Sora 録画が fMP4 入力に対応する場合（issue 0001 段階 2a）の前借りになる。
- テスト: `tests/e2e.rs` の `inspect_fragmented_*` の `format` 期待値を `"fmp4"` に修正する（format アサーションが無い fragmented テストには追加する）。
- CHANGES.md: fMP4 対応エントリに「inspect は fMP4 を `format: "fmp4"` として返す」を反映する。

## 実装上の依存

- 実装は `feature/add-fmp4-read-support` 上で、fMP4 機能がマージ・リリースされる前に行う。develop 単体では inspect が fMP4 を扱えない（`Mp4SampleReader` / `detect_mp4_file_kind` が未マージ）ため。

## 完了条件

- fMP4 を inspect したとき `format` が `"fmp4"` になり、通常 MP4 は `"mp4"`、WebM は `"webm"` のままであること。
- `ContainerFormat::Fmp4` が Sora 録画メタデータのパース経路では生成されないこと。
- e2e テストで fMP4 の `format == "fmp4"` を検証すること。
- CHANGES.md に反映すること。

## 備考

- fMP4 機能のリリース後にこの対応を行う場合、`"mp4"` を一度リリースしてから変えることになり [CHANGE]（後方互換破壊）になる。リリース前に入れるのが望ましい。
