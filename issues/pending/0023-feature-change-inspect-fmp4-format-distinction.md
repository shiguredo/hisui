# inspect の format で fMP4 を通常 MP4 と区別するか検討する

- Priority: Low
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## pending の理由

fMP4 を区別するか、区別する場合の表現方法（`format` 値を変える / 別フィールドを足す）が未決の設計判断であり、`format` の値を変える案は後方互換に影響する。方針が決まるまで pending とする。

## 目的

現状 inspect は fMP4 入力でも `format` フィールドを `"mp4"` として返し、通常 MP4 と区別しない。区別したい要望が出た場合に備え、区別の要否と方法を検討する。

## 優先度根拠

Low。現状の `"mp4"` 固定で実害はなく、区別の要望が顕在化してから検討すればよい。

## 現状

- `src/types.rs` の `ContainerFormat` は `Mp4` / `Webm` の 2 値のみ（`Fmp4` なし）。`ContainerFormat::from_path` は拡張子だけで判定し、ファイル実体（ftyp / moov）を見ない。
- inspect の `format` フィールドは `ContainerFormat` の JSON 出力（`Mp4 => "mp4"`）で、`--decode` の有無に関わらず fMP4 入力でも `"mp4"`。e2e テスト `inspect_fragmented_mp4_video_only` が `format == "mp4"` を assert 済み。
- これは issue 0001 で「段階 1 では mp4 と fmp4 を区別しない。区別したい要望が出てきたら別途検討する」と明示的に先送りされた項目。

## 設計方針（検討対象）

- 案 A: 現状維持（`"mp4"` 固定）。
- 案 B: fMP4 のとき `format` を `"fmp4"` にする（`format` 値の後方互換が壊れ、消費側に影響する）。
- 案 C: `format` は `"mp4"` のまま、別フィールド（例: `fragmented: true`）で示す（後方互換を保てる）。
- いずれの区別案も、実体判定（既存の `detect_mp4_file_kind`）を inspect 経路で使う必要がある。

## 完了条件

- 区別の要否を決定すること。区別する場合は方法（B / C 等）と後方互換方針を定め、CHANGES.md に記載すること。
