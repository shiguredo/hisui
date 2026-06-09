# sample_entry フィールドを非 Option 化する

- Priority: Low
- Created: 2026-06-08
- Completed: 2026-06-09
- Model: Claude Opus 4.8
- Branch:
- Polished:

## 目的

issue 0017（音声）と issue 0027（映像）が完了すると、音声・映像とも全出力フレームに sample_entry が載るようになる。その状態では `AudioFrame.sample_entry` / `VideoFrame.sample_entry` の `Option` は常に `Some` となり、`None` の状態が存在しなくなる。

本 issue では、これらのフィールドを `Option<SharedSampleEntry>` から `SharedSampleEntry`（非 Option）に変更し、型レベルで「sample_entry は必ず存在する」ことを保証する。これにより下流（各 writer）の `Option` 分岐や `or_else` 補完が不要になり、コードが簡潔になる。

**前提**: 本 issue は issue 0027 を先に完了する必要がある。非 Option 化には映像が全フレーム付与になっていることが必須で、それは 0027 で実現される（音声側は 0017 が前提）。0027 未完了では着手できない。

## 優先度根拠

Low。機能的なバグ修正ではなく、型の最終形を整える仕上げ。0017・0027 の完了後に Option を外して型を締める。時間があるときに対応する。

## 現状

- `AudioFrame.sample_entry`（`src/audio.rs:84-91`）と `VideoFrame.sample_entry`（`src/video.rs:44-51`）は、0017・0027 完了後に `Option<SharedSampleEntry>` となっている。
- 0017・0027 完了後は全フレームに sample_entry が載るため、`None` は実際には発生しない。だが型は `Option` のままで、「いつ `None` か」の曖昧さが残る。
- 各 writer は `or_else(|| last_*_sample_entry.clone())` 等の `Option` 補完ロジックを持つ。全フレーム付与後はこれらが不要になる。

## 設計方針

- `AudioFrame.sample_entry` / `VideoFrame.sample_entry` を `SharedSampleEntry`（非 Option）に変更する。
- 全エンコーダが「最初の出力フレームで必ず sample_entry を確定する」不変条件を満たすことを確認する（音声は `new()` で確定、映像は最初の出力フレームで確定）。確定できない初回フレームの扱い（`expect("MESSAGE")` でパニックさせるか、そもそも発生しないことの保証か）を整理する。
- 各 writer の `Option` 分岐・`or_else` 補完を除去し、`changed_since` ベースの変更検知に一本化する。
- writer 側で「前回 entry」を保持する変数（`last_*_sample_entry`）は、初回判定のため `Option<SharedSampleEntry>` のまま残る点に注意する。

## 完了条件

- `AudioFrame` / `VideoFrame` の sample_entry フィールドが非 Option になること。
- 全エンコーダ・全 writer が非 Option フィールドでコンパイル・動作すること。
- 「最初の出力フレームで entry が確定する」不変条件が全エンコーダで満たされることを確認すること。
- 非 Option 化後も全エンコーダ・全 writer が正しく動作することを単体テスト・統合テストで検証する（本リポジトリには PBT 基盤（pbt クレート・proptest）が無いため、`tests/*_tests.rs` と `#[cfg(test)]` 単体テストで行う。issue 0017 の「テスト」節と同じ方針）。
- 録画機能にリグレッションが無いこと。

## 関連

- issue 0017（音声側の全フレーム付与と共通型導入。間接的な前提）
- issue 0027（直接の前提。映像の全フレーム付与統一。本 issue は 0027 完了後に着手する）

## クローズ理由（2026-06-09・実装せず close）

本 issue の核心的前提「0017・0027 完了後は `AudioFrame.sample_entry` / `VideoFrame.sample_entry` が常に `Some` になり `None` が消える」は誤りだったため、実装せず close する。

- `AudioFrame`（`src/audio.rs`）/ `VideoFrame`（`src/video.rs`）は、デコード → ミックス → エンコード → muxer のパイプライン全体で共有される単一型である（エンコーダのシグネチャは `fn encode(&mut self, frame: &AudioFrame) -> Result<AudioFrame>`。`src/encoder/opus.rs` 参照）。sample_entry はエンコーダ出力フレームでのみ意味を持つ。
- 生データ・デコーダ出力・ミキサ出力など、sample_entry が意味を持たないフレームの構築サイトが 30 箇所超あり（`src/decoder/*.rs`、`src/mixer/*.rs`、`src/obsws/source/*.rs`、`src/webm/reader.rs`、`src/video.rs` の生データ構築など。コメント「生データにはサンプルエントリは存在しない」）、これらは 0017・0027 完了後も `None` のまま正しい。よって sample_entry フィールドが `None` になる状態は構造的に消えない。
- フィールドを非 Option にするには「生フレーム型（sample_entry 無し）」と「エンコード済みフレーム型（非 Option）」をパイプライン全体で型分割する大改修が必要になる。得られる便益（writer の `Option` 分岐・`or_else` 補完の除去）に対して改修規模が著しく不釣り合いで、CLAUDE.md「Premature Optimization is the Root of All Evil」にも反する。
- なお 0027 の磨き上げ時点で「生データ由来の `VideoFrame` は `None` のまま正しい」ことは把握しており、その事実が本 issue の前提を覆した。

将来どうしても非 Option 化したくなった場合は、本 issue ではなく「`RawFrame` / `EncodedFrame` の型分割」を主目的とする別 issue として、優先度と波及範囲を見直したうえで起票し直すこと。
