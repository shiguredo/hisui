# 音声 sample_entry の欠落による finalize 失敗（映像トラック空）を直す

- Priority: High
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished: 2026-06-08

## 目的

短時間録画等で StopRecord 後の出力 MP4 の映像トラックが空になることがある問題（issues/0011）の真因を直す。録画した映像が再生できなくなるデータ破損であり、実害が大きい。

## 優先度根拠

High。録画映像が再生不能になるデータ破損で、CI の e2e でも再発が確認されている（発生率は約 3% 程度）。録画機能の信頼性に直結する。

## 現状（真因）

- finalize 時に muxer が「Missing sample entry for first sample of Audio track」で Err を返し、標準 MP4 への変換が失敗する。失敗時は出力が録画中の fMP4 形式のまま残り、録画全体が単一の未 flush フラグメントに収まる短時間録画では映像トラックが空になる。
- muxer（`shiguredo_mp4` の `mux`）は「トラックの最初のサンプルに sample_entry が必須」という契約を持つ（finalize 時に最初のサンプルの sample_entry が None だと上記エラーを返す）。
- 真因: 音声エンコーダ（`OpusEncoder` / `FdkAacEncoder` / `AudioToolboxEncoder`）は sample_entry を最初の出力フレームにしか載せない（`self.sample_entry.take()`、`src/encoder/opus.rs:58` / `src/encoder/fdk_aac.rs:81` / `src/encoder/audio_toolbox.rs:175`）。
- そのため録画 writer が最初の entry 付き音声フレームを取りこぼすと（合流タイミング・起動レース等）、sample_entry が一度も届かず `last_audio_sample_entry` が None のまま finalize に至り失敗する。
- 映像は「録画開始時のキーフレーム要求」＋「keyframe には sample_entry を常に補完」（`src/encoder.rs` の `push_encoded_frame_with_metrics`、`src/encoder.rs:724-739`）で確実に entry が届くが、音声には同等の機構が無い。
- issues/0011 で入れた「writer 入口（`handle_*_message`）での sample_entry 取り込み」は、届いた entry を pause 等の drop で落とさないための hardening であり、「そもそも届かない」本症状は塞げていない（CI の e2e `obsws/test_output.py::test_obsws_srt_inbound_with_stream_id` で再発を確認済み）。

## 設計方針

音声エンコーダが sample_entry を「最初の 1 フレームだけ」載せる構造がレースの本質。これを「全フレームに載せる」に変えて、いつ subscribe しても次フレームで必ず entry が届くようにし、レースのカテゴリ自体を消す。あわせて、毎フレームの sample_entry のコピーと変更検知を安価にするため共通型を導入する。

### 決定事項

- 決定 1: 音声先行 + 型共通化。音声フレームを全フレーム付与に変えてバグを直す。共通型 `SharedSampleEntry` を導入し映像のフィールド型も揃えるが、映像の挙動（keyframe 補完）は据え置き。映像の全フレーム化・非 Option 化は別 issue に切る（バグ修正の粒度を保つため）。
- 決定 2: 変更検知 `changed_since` は `Arc::ptr_eq`（fast path）＋ `PartialEq`（別 Arc 時の実体比較 fallback）の二段とする。生成側の Arc 同一性保証に依存せず、writer が常に正確に判定できる。`shiguredo_mp4::boxes::SampleEntry` は `PartialEq` / `Eq` を実装済み。
- 決定 4: muxer のインターフェースは変えず、writer 入口の変更検知フィルタで「変化時のみ」 entry を muxer に渡す。これにより muxer から見える entry の出方は現状互換（最初は必ず Some、以降は変化時のみ Some）になり、muxer の同値集約挙動には依存しない（安全側）。

（決定 3 = 映像の補完責務、決定 5 = `SharedSampleEntry` の定義場所は、それぞれ映像統一の別 issue / 実装着手時に確定する。）

### 共通型

```rust
/// 映像・音声で共有する sample entry。
/// Arc で包むことで毎フレームのコピーを避け、変化検知を ptr_eq で安価に行う。
#[derive(Debug, Clone)]
pub struct SharedSampleEntry(Arc<SampleEntry>);

impl SharedSampleEntry {
    pub fn new(entry: SampleEntry) -> Self;
    pub fn get(&self) -> &SampleEntry;

    /// 直前の entry から変化したかを判定する。
    /// ptr_eq が同一なら短絡して実体比較を省き、別 Arc のときだけ PartialEq で確認する。
    pub fn changed_since(&self, prev: Option<&SharedSampleEntry>) -> bool;
}
```

### 実装スコープ

1. 新規 `SharedSampleEntry(Arc<SampleEntry>)`（`new` / `get` / `changed_since`）を追加する。
2. `AudioFrame.sample_entry` / `VideoFrame.sample_entry` を `Option<SampleEntry>` から `Option<SharedSampleEntry>` に変更する。
3. 全エンコーダの sample_entry 生成箇所を `SharedSampleEntry::new(...)` でラップする。
4. 音声 3 エンコーダ（`opus` / `fdk_aac` / `audio_toolbox`）の `self.sample_entry.take()` を廃止し、毎フレーム `Some(self.sample_entry.clone())`（Arc clone）を載せる。これがバグ修正の核心。
5. 全 writer（`hls` / `dash` / `mp4` / `mp4/hybrid`）で、音声フレームの sample_entry を `changed_since` で判定し、変化時のみ muxer に渡す。`last_audio_sample_entry` も `SharedSampleEntry` 型にする。
6. 映像は挙動据え置き（型ラップのみ）。副次的に `last_video_sample_entry` の clone が Arc clone になり軽くなる。

### 非対象

- 映像の全フレーム付与・非 Option 化・Arc 同一性保証（別 issue）。
- sample_entry フィールドの非 Option 化（映像の全フレーム付与とセットのため別 issue）。
- mp4 writer 側でコーデック情報から sample_entry を合成する案（writer の責務外）。

## 完了条件

- 短時間 SRT 録画（`obsws/test_output.py::test_obsws_srt_inbound_with_stream_id` および `obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output`）を CI で十分な回数繰り返しても finalize 失敗が再発しないこと。発生率が約 3% のため、issues/0008 で用いた 100 回相当（10 シャード × 10 回）の一時ワークフローで検証し、検証後にそのワークフローは削除する。
- 検証には観測メトリクス（`hisui_total_finalize_failure_count` / `hisui_total_missing_audio_sample_entry_count` / `hisui_total_received_audio_sample_entry_count`）と warn ログ（`Missing sample entry for first sample of Audio track`）を使う。これらは feature/fix-hybrid-writer-finalize-on-stop-record で追加済み。
- なお全フレーム付与に伴い `hisui_total_received_audio_sample_entry_count` の計数タイミング（writer の変更検知フィルタの前後どちらで数えるか）が指標の意味を変えるため、実装時に計数位置を確認し意図を明確にすること。
- 音声・映像の sample_entry ラウンドトリップを PBT で検証する。
- 修正完了後に issues/0011 を close する。

## 関連

- issues/0011（reopen 済み。本 issue で真の修正を行う）
- issues/0008（先行する flaky テストの issue。closed）
- 別 issue（映像側の sample_entry を `SharedSampleEntry` で全フレーム付与・非 Option 化に統一するリファクタリング。本 issue 完了後に作成する）
