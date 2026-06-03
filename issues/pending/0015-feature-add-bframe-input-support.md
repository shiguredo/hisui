# B フレーム (composition_time_offset) を含む入力ファイルの読み込みに対応する

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-bframe-input-support
- Polished:

## 目的

現状 hisui は MP4 / fMP4 の前方読みパスで `composition_time_offset` (B フレーム由来の CTS オフセット) を持つサンプルを未対応として弾いている。配信用 fMP4 や一般的な H.264 / H.265 ファイルは B フレームを使うことが多く、外部ツール由来のファイルを読み込もうとするとこの制約に当たりやすい。本対応では、B フレームを含む入力 (composition_time_offset を持つサンプル) を読み込めるようにする。

## pending とした理由

- composition_time_offset の扱いは前方読みパス全体に関わる横断的な課題であり、CTS と DTS / PTS の区別、サンプルの並べ替え (デコード順 vs 表示順)、合成パイプラインのタイムスタンプ計算 (`src/timestamp/`)、合成タイミングへの影響など、複数箇所にまたがる設計判断が必要。
- このため即着手せず、設計の方向性が固まるまで `issues/pending/` で保留する (AGENTS.md「外部依存の追加や設計判断が必要で保留中の issue は issues/pending/ に置く」)。pending の issue は修正せずそのまま残す。

## 優先度根拠

- 外部ファイルの持ち込みで実際に詰まるが、Sora 純正の録画は B フレーム無しのため業務を直撃しているわけではない。High ではなく Medium。

## 現状

- 読み込み側で composition_time_offset を持つサンプルを明示的にエラーにしている箇所:
  - `src/mp4/reader.rs:1010-1013` — `"composition_time_offset is not supported yet"`。
  - `src/sora/recording_mp4_reader.rs:93-96` および `:245-248` — 同上 (音声 / 映像 reader)。
  - `src/mp4/reader.rs:1428` / `:1443` — サンプルコンテキストは composition_time_offset を保持はしているが活用していない。
- 書き込み側は全 writer が常に `composition_time_offset: None` を出力する (`src/mp4/writer.rs` / `hybrid_writer.rs` / `hls/writer.rs` / `dash/writer.rs`)。出力側の B フレーム対応 (CTS を書き出す) は本 issue のスコープ外とし、入力 (読み込み) に絞る。
- issue 0001 (fMP4 read support) でも、段階 1 / 2 を通じて composition_time_offset は一貫して非対応とし、「前方読みパス全体に関わる横断的な課題のため将来の別 issue」として繰り返し先送りされている。本 issue がその follow-up。

## 設計方針 (要検討・未確定)

設計の方向性は未確定。最低限、次を詰める:

- composition_time_offset を使った PTS (表示時刻) の算出と、既存の DTS ベースのタイムスタンプ計算 (`src/timestamp/`) との整合。
- 前方読み (next_sample) でデコード順に来るサンプルを、合成側が表示順で扱えるようにする方法 (リオーダリングの要否と責務の所在)。
- decoder (`src/decoder/`) が B フレームを含むストリームを正しく扱えるかの確認。
- inspect / 録画合成 / OBSWS の各経路で必要範囲が異なるため、issue 0001 の段階分けと同様に段階化するか。

## 完了条件

- B フレームを含む入力 (composition_time_offset を持つサンプル) を、少なくとも inspect で正しく読み取れること (具体的な完了条件は設計確定後に詰める)。
- 既存の B フレーム無しファイルの読み込みに回帰がないこと。

(解決方法は pending のため、設計確定後に記載する。)
