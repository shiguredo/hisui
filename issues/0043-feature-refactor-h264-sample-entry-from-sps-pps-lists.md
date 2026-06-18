# h264_sample_entry_from_annexb を SPS / PPS リスト受け取り版にリファクタして NAL 走査の二重化と avc_profile_indication / avc_level_indication 固定値 TODO を解消する

- Priority: Low
- Created: 2026-06-18
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-h264-sample-entry-from-sps-pps-lists
- Polished: {YYYY-MM-DD}

## 目的

issue 0037 で SRT inbound 経路に SPS 解像度抽出を導入した結果、`src/srt/inbound_endpoint.rs:929-948` の `build_video_sample` で「IDR 判定 + SPS NAL 収集」を 1 回走査したあとに `h264_sample_entry_from_annexb` を呼び出し、関数内 (`src/video/h264.rs:87-129`) で再度 `H264AnnexBNalUnits` を走査して SPS / PPS を抽出する構造になっており、同じバイト列に対する NAL 走査が事実上 2 回行われている。同関数の `Avc1Box.avcc_box.avc_profile_indication` / `avc_level_indication` は `src/video/h264.rs:118-119` で `H264_PROFILE_BASELINE` / `H264_LEVEL_3_1` の固定値 + `TODO: 実際の値に合わせる` コメントのまま残っている。

issue 0037 で `extract_dimensions_from_sps` を整備して SPS から profile_idc / level_idc を取り出せる経路が既に存在する。本 issue では:

1. `h264_sample_entry_from_annexb` を「SPS / PPS リスト受け取り版」にリファクタリングして NAL 走査の二重化を解消する。
2. SPS バイト列から profile_idc / level_idc / chroma_format / bit_depth を取り出して `avcC` ボックスの該当フィールドに反映し、固定値 TODO を解消する。

## 優先度根拠

Low。本 issue は内部効率化（走査 1 回への削減）と broken window 解消（固定値 TODO）が主目的で、外部から観測可能な機能変更は伴わない。NAL 走査 2 回でも実害は無いし、`avc_profile_indication` が Baseline 固定でも下流（プレイヤー / 別 MP4 ツール）の互換性で大きな問題は出ていない。ただし issue 0037 完了で SPS パース経路が整ったため、自然に解消できる位置付け。

## 現状

行番号は HEAD（develop = 40878768）時点。実装着手時は grep で再特定する。

### NAL 走査の二重化

- `src/srt/inbound_endpoint.rs:927-948`: `build_video_sample` で `H264AnnexBNalUnits` を 1 回走査して IDR 判定 + SPS NAL 収集 (`sps_nal: Option<&[u8]>`)。その後 `h264_sample_entry_from_annexb(width, height, &pending.data)` (`:948`) を呼ぶ。
- `src/video/h264.rs:87-129`: `h264_sample_entry_from_annexb` 内部で再度 `H264AnnexBNalUnits::new(data)` を走査して `sps_list` / `pps_list` を構築。

つまり同じ `pending.data` に対する NAL 走査が 2 回行われる。

### 固定値 TODO

- `src/video/h264.rs:118-119`:
  ```rust
  avc_profile_indication: H264_PROFILE_BASELINE, // TODO: 実際の値に合わせる
  avc_level_indication: H264_LEVEL_3_1,          // TODO: 実際の値に合わせる
  ```
  `H264_PROFILE_BASELINE = 66` / `H264_LEVEL_3_1 = 31` で固定。SPS バイト列の先頭 3 バイト（profile_idc / constraint_set + reserved / level_idc）から取り出せる値。

### 呼び出し側

- `src/encoder/nvcodec.rs:50`: `h264::h264_sample_entry_from_annexb(width, height, &seq_params)?;`
- `src/encoder/openh264.rs:62`: 同上
- `src/decoder/openh264.rs:167, :201`: 同上
- `src/srt/inbound_endpoint.rs:948`: 同上

いずれも「encoder / decoder の seq_params or RTSP / SRT inbound の PES 全体」を渡す形。

### 既存のテスト

- `src/video/h264.rs` の `#[cfg(test)] mod tests`: `extract_dimensions_from_sps` の単体テスト群（issue 0037 で整備）
- `src/srt/inbound_endpoint.rs` の `#[cfg(test)] mod tests`: SRT inbound の build_video_sample テスト
- `tests/decoder_tests.rs` 等の encoder / decoder 経由のテスト

## 設計方針

### スコープ

- NAL 走査二重化の解消は **SRT inbound 経路** が主対象。encoder / decoder 経路は呼び出し側で SPS / PPS リストを既に持っていない場合が多いため、走査 1 回化の恩恵は小さい（既存挙動維持）。
- 固定値 TODO 解消は **全経路** が対象（`h264_sample_entry_from_annexb` の戻り値が影響する）。

### 関数構成

以下 2 段構えにする:

1. **新ヘルパー関数** `h264_sample_entry_from_sps_pps_lists(sps_list, pps_list) -> Result<SampleEntry>`:
   - 入力: `Vec<Vec<u8>>` の SPS リスト / PPS リスト（NAL ヘッダ含む raw NAL）
   - 内部で sps_list[0] をパースして profile_idc / level_idc / chroma_format / bit_depth を取り出し、`Avc1Box` に反映
   - width / height は SPS の `extract_dimensions_from_sps` 相当の処理で取得して `visual` に反映（既存の引数 width / height は不要になる）
2. **既存関数** `h264_sample_entry_from_annexb(data) -> Result<SampleEntry>`:
   - 内部で `H264AnnexBNalUnits` を走査して SPS / PPS リストを構築
   - 新ヘルパー関数を呼ぶ薄いラッパー
   - 引数の `width` / `height` は削除（呼び出し側で SPS 由来の値を使う）

### `extract_dimensions_from_sps` との関係

`extract_dimensions_from_sps` の内部処理（SPS バイト列を読んで profile_idc / level_idc / chroma_format_idc / bit_depth / pic_width / pic_height / cropping を抽出）と、新ヘルパー関数で必要な avcC フィールド抽出は **大部分が重複** する。

選択肢:

- 案 A: `extract_dimensions_from_sps` を `parse_sps(sps) -> Result<SpsParams>` のような全パラメータ抽出関数に拡張し、`extract_dimensions_from_sps` は薄いラッパーにする。新ヘルパー関数は `parse_sps` の結果を avcC に詰める。
- 案 B: `parse_sps_profile_level(sps) -> Result<(profile_idc, level_idc, chroma_format, bit_depth_*)>` のような小さなヘルパーを別途切り出し、`extract_dimensions_from_sps` と新ヘルパー関数で個別に呼ぶ。

案 A の方が SPS パース処理が一箇所にまとまるため整理しやすい。実装着手時に確定する。

### 呼び出し側の変更

- `src/srt/inbound_endpoint.rs:947-948`: 1 回目の NAL 走査で SPS / PPS リストを構築するように拡張し、新ヘルパー関数を直接呼ぶ。これにより走査 1 回化が完成。
- `src/encoder/nvcodec.rs:50` / `src/encoder/openh264.rs:62` / `src/decoder/openh264.rs:167, :201`: 既存関数 `h264_sample_entry_from_annexb(data)` を引き続き呼ぶ（引数 `width, height` を削除した形に追従）。挙動変化は「avcC の profile_indication / level_indication が SPS 由来の実値になる」のみ。

### `extract_video_dimensions` (`src/video/h264.rs:132`)

既存関数 `extract_video_dimensions(entry) -> Result<(u32, u32)>` は AVC1 サンプルエントリーから width / height を取り出す関数で、本 issue のリファクタとは別物。命名衝突しないように注意する。

## 完了条件

- `src/video/h264.rs` に新ヘルパー関数 `h264_sample_entry_from_sps_pps_lists` (または同等の関数) が追加され、`h264_sample_entry_from_annexb` は新関数の薄いラッパーになっている。
- `src/srt/inbound_endpoint.rs:927-948` の `build_video_sample` で `pending.data` に対する NAL 走査が 1 回だけになり、`H264AnnexBNalUnits` の走査は IDR 判定 + SPS / PPS 収集の 1 ループに統一されている。
- `src/video/h264.rs:118-119` の `avc_profile_indication: H264_PROFILE_BASELINE, // TODO: ...` / `avc_level_indication: H264_LEVEL_3_1, // TODO: ...` が SPS 由来の実値（profile_idc / level_idc）に置き換わり、TODO コメントが消えている。
- `Avc1Box.avcc_box.chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` も SPS 由来の実値で `Some(...)` 化される（仕様 7.4.2.1.1 の High プロファイルでは SPS から取得、Baseline / Main / Extended ではデフォルト値）。
- 既存テスト（encoder / decoder 経由のテスト含む）が全て pass する。
- 新ヘルパー関数で構築した `SampleEntry::Avc1` の `avcc_box.avc_profile_indication` / `avc_level_indication` が SPS 由来の実値になっていることを確認する単体テストが追加されている。
- `cargo test` / `cargo clippy --all-targets -- --deny warnings` / `cargo fmt --all -- --check` がパスする。

### CHANGES.md

`avc_profile_indication` / `avc_level_indication` が固定値から SPS 由来の実値に変わるため、生成される MP4 のメタデータが入力ストリーム依存になる。これは外部から観測可能な挙動変化であり、`[UPDATE]` 相当として CHANGES.md `## develop` に記載する想定（記載文言・形式は実装時に `shiguredo-changelog` 規約に照らして確定）。

## 解決方法

実装着手後にここに記述する。

## 関連

- issue 0037 (closed 想定): SPS 解像度抽出パーサ `extract_dimensions_from_sps` を追加。本 issue の前提となる SPS パース経路を提供。
- issue 0030 (closed): エンコード済み圧縮フレームの `sample_entry` 不変条件確立。
- issue 0034 (closed): writer 入口の `sample_entry` 不変条件違反検知 + fallback 補完。
- issue 0039 (open): writer 側 fallback 経路の削除可否調査。本 issue とは独立だが、`sample_entry` 周りの整理として並行 / 後続で進む。
