# sample_entry フィールドを非 Option 化する

- Priority: Low
- Created: 2026-06-08
- Completed:
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
- PBT で sample_entry のラウンドトリップを検証する。
- 録画機能にリグレッションが無いこと。

## 関連

- issue 0017（音声側の全フレーム付与と共通型導入。間接的な前提）
- issue 0027（直接の前提。映像の全フレーム付与統一。本 issue は 0027 完了後に着手する）
