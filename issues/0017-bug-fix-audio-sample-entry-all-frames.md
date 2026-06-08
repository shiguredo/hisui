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

High。録画映像が再生不能になるデータ破損で、CI の e2e でも再発が確認されている。録画機能の信頼性に直結する。発生率（約 3%）の出典と母数は「現状（真因）」に記す。

## 現状（真因）

- finalize 時に muxer が「Missing sample entry for first sample of Audio track」で Err を返し、標準 MP4 への変換が失敗する。失敗時は出力が録画中の fMP4 形式のまま残り、録画全体が単一の未 flush フラグメントに収まる短時間録画では映像トラックが空になる。
- 真因: 音声エンコーダ（`OpusEncoder` / `FdkAacEncoder` / `AudioToolboxEncoder`）は sample_entry を最初の出力フレームにしか載せない（`self.sample_entry.take()`、`src/encoder/opus.rs:58` / `src/encoder/fdk_aac.rs:81` / `src/encoder/audio_toolbox.rs:175`）。
- そのため録画 writer が最初の entry 付き音声フレームを取りこぼすと（合流タイミング・起動レース等）、sample_entry が一度も届かず、writer の `last_audio_sample_entry` が None のまま finalize に至り失敗する。映像には「録画開始時のキーフレーム要求」＋「keyframe への sample_entry 補完」（`src/encoder.rs:724-739`）があるため確実に届くが、音声には同等の機構が無い。
- issues/0011 で入れた「writer 入口（`handle_*_message`）での sample_entry 取り込み」は、届いた entry を pause 等の drop で落とさないための hardening であり、「そもそも届かない」本症状は塞げていない（CI の e2e `obsws/test_output.py::test_obsws_srt_inbound_with_stream_id` で再発を確認済み）。
- 発生率: issues/0008 のフェーズ A で対象テストを CI で 100 回繰り返し、約 3%（3/100 前後）でモード 1（inspect が映像トラックを読めない）を再現した。詳細は issues/0008 の「## 解決方法 / 結論」を参照。

### muxer の sample_entry 契約（一次確認）

`shiguredo_mp4` `=2026.3.0` の muxer（`mux_mp4_file.rs`）の実挙動を実コードで確認した。本 issue の設計判断の前提になるため記す（このクレートの内部実装であり将来変更されうる）。

- 各トラックの「最初のサンプル」には sample_entry が必須。チャンク生成時に sample_entry が無く、かつ過去チャンクも無いと `MuxError::MissingSampleEntry` を返す（`mux_mp4_file.rs:585-592`）。これが本バグのエラー源。
- 2 サンプル目以降、muxer は受け取った sample_entry が直前チャンクと同値かを `PartialEq` で比較し（`is_new_chunk_needed`、`mux_mp4_file.rs:611-628`）、同値なら新チャンクを作らず無視する。sample_entry が None のサンプルも「前と同じ」として扱い無視する。
- つまり「全フレームに `Some(entry)` を載せて muxer に丸投げ」しても、muxer 側で正しく集約され、出力 MP4 は現状と同一になる。**最初のサンプルに必ず entry が届きさえすれば本バグは解消する。**

この事実は後述の設計判断（writer のフィルタは correctness 要件ではなく optimization）の根拠になる。

## 設計方針

音声エンコーダが sample_entry を「最初の 1 フレームだけ」載せる構造がレースの本質。これを「全フレームに載せる」に変えて、いつ subscribe しても次フレームで必ず entry が届くようにし、レースのカテゴリ自体を消す。これがバグ修正の核心であり、これ単独で症状は解消する（上記 muxer 契約より）。

あわせて、音声側の sample_entry の型として共通型 `SharedSampleEntry` を導入する。これは後続の issue 0027（映像の全フレーム付与）/ 0028（非 Option 化）でも使う土台であり、本 issue 単独のためだけの最適化ではない。

### 決定事項

- 決定 1: 音声先行 + 差分の最小化。音声フレームを全フレーム付与に変えてバグを直す。共通型 `SharedSampleEntry` を導入して `AudioFrame.sample_entry` の型を `Option<SharedSampleEntry>` にするが、`VideoFrame.sample_entry` は `Option<SampleEntry>`（生の型）のまま据え置く。映像のフィールド型統一・全フレーム化・非 Option 化はすべて別 issue（0027 / 0028）で行い、本 issue では映像側のコードに一切手を入れない（バグ修正の粒度を保ち、差分を最小化するため）。音声と映像でフレーム型が一時的に不揃いになるが、両者は独立したフィールドで writer の処理経路も別なので問題なく、0027 で揃う。
- 決定 2: 変更検知 `changed_since` は `Arc::ptr_eq`（fast path）＋ `PartialEq`（別 Arc 時の実体比較 fallback）の二段とする。生成側の Arc 同一性保証に依存せず、writer が常に正確に判定できる。`shiguredo_mp4::boxes::SampleEntry` は `PartialEq` / `Eq` を実装済み（`boxes_sample_entry.rs` の derive で確認）。
- 決定 3: `changed_since` メソッド自体は `SharedSampleEntry` に必ず実装する（後続の issue 0027 がこれを前提とするため）。一方、writer 側で「変化時のみ muxer に渡す」フィルタとして適用するかは optional とする。フィルタは correctness 要件ではなく、(a) muxer から見える entry の入力パターンを現状互換に保つ安全策、(b) muxer 側の毎フレーム `PartialEq` 比較を省く最適化、の二点が目的であり、muxer 契約（上記）よりフィルタが無くても出力は正しい。なお `received` カウンタの計上は、このフィルタ適用の有無とは独立に入口で行う（完了条件を参照）。
- 決定 4: `SharedSampleEntry` を `Arc` で包むのは音声・映像で毎フレーム値コピーを避けるためだが、`SampleEntry` は小さい値型（enum）で値 clone のコストは実測上ほぼ誤差である（CLAUDE.md「性能差は誤差程度」「Premature Optimization is the Root of All Evil」）。よって Arc 化は性能最適化としてではなく、(a) 0027 / 0028 で全フレーム付与・非 Option 化したときに毎フレーム clone を Arc clone にして将来のコピー増を抑える布石、(b) `changed_since` の ptr_eq fast path を提供する設計、として位置づける。性能を理由に正当化しない。

### 共通型

```rust
/// 映像・音声で共有する sample entry。
/// Arc で包むことで毎フレームのコピーを Arc clone に抑え、変化検知を ptr_eq で短絡できる。
#[derive(Debug, Clone)]
pub struct SharedSampleEntry(Arc<SampleEntry>);

impl SharedSampleEntry {
    pub fn new(entry: SampleEntry) -> Self;
    pub fn get(&self) -> &SampleEntry;

    /// 直前の entry から変化したかを判定する。
    /// prev が None（初回）なら true（=変化あり扱い）。
    /// 同一 Arc なら ptr_eq で短絡して false（変化なし）。
    /// 別 Arc のときだけ PartialEq で実体比較し、相違なら true・同値なら false を返す。
    pub fn changed_since(&self, prev: Option<&SharedSampleEntry>) -> bool;
}
```

定義場所: 音声・映像の双方から参照するため、crate ルート（`src/lib.rs` もしくは新規 `src/sample_entry.rs`）に置く。実装着手時に既存の型配置に合わせて最終決定する。

### 実装スコープ

1. 新規 `SharedSampleEntry(Arc<SampleEntry>)`（`new` / `get` / `changed_since`）を追加する。
2. `AudioFrame.sample_entry`（`src/audio.rs:90`）のみを `Option<SampleEntry>` から `Option<SharedSampleEntry>` に変更する。`VideoFrame.sample_entry`（`src/video.rs:50`）は触らない（映像のフィールド型統一は issue 0027 で行う）。
3. `AudioFrame.sample_entry` の型変更に伴う波及を、音声フレームを生成・消費・中継する箇所で吸収する。読む側は `.as_ref().map(|e| e.get())` 等で `&SampleEntry` を取り出し、生成側は `SharedSampleEntry::new(...)` でラップする。波及先は音声エンコーダ（`src/encoder/{opus,fdk_aac,audio_toolbox}.rs`）、音声デコーダ（`src/decoder/{fdk_aac,audio_toolbox}.rs`）、reader（`src/mp4/{reader,sample_reader}.rs`、`src/sora/recording_mp4_reader.rs`）、`src/rtmp/frame.rs`、`src/rtsp/subscriber.rs`、`src/srt/inbound_endpoint.rs` の音声経路。**feature gate された箇所（`fdk-aac` の `src/decoder/fdk_aac.rs` 等）はデフォルトの `cargo check` では検出されないため、`--features fdk-aac` でも必ずビルド確認すること**（CI の `test-fdk-aac` で検出される）。
4. 音声 3 エンコーダ（`opus` / `fdk_aac` / `audio_toolbox`）の sample_entry 生成箇所を `SharedSampleEntry::new(...)` でラップし、`self.sample_entry.take()` を廃止して毎フレーム `Some(self.sample_entry.clone())`（Arc clone）を載せる。これがバグ修正の核心。
5. 各 writer の音声経路のみを対応する（映像経路は触らない。補完経路が writer ごとに非対称なため一律ではない）:
   - `mp4/hybrid_writer.rs`: `last_audio_sample_entry` を `Option<SharedSampleEntry>` にする（`last_video_sample_entry` は生の `Option<SampleEntry>` のまま）。`append_audio_to_fragment` の muxer 渡しでは `.get().clone()` で生の `SampleEntry` を取り出す。`or_else(|| self.last_audio_sample_entry.clone())` 補完は当面残す。
   - `dash/writer.rs` / `hls/writer.rs`: `last_audio_sample_entry`（生の `SampleEntry`）はそのまま保持し、音声フレームの `frame.sample_entry`（`SharedSampleEntry`）を `.get()` 経由で読む。HLS は `wrap_raw_aac_in_adts` / `fill_missing_sample_entries` が保持済みの生 `last_audio_sample_entry` を使うため、これらのシグネチャは変更不要。
   - `mp4/writer.rs`（標準 Mp4Writer）: 音声サンプルの muxer 渡し（`src/mp4/writer.rs:788` 付近）を `frame.sample_entry.as_ref().map(|e| e.get().clone())` に直す。
   - `received_audio_sample_entry` カウンタは入口で `changed_since` が true のときだけ計上する（完了条件を参照）。
6. テストの更新（後述の「テスト」を参照）。

### 非対象

- 映像側の sample_entry に関する一切（`VideoFrame.sample_entry` のフィールド型統一・全フレーム付与・非 Option 化、映像エンコーダ/デコーダ/writer の映像経路）。すべて issue 0027 / 0028 で行い、本 issue では映像コードに手を入れない。
- mp4 writer 側でコーデック情報から sample_entry を合成する案（writer の責務外）。
- PBT 基盤（proptest 依存・pbt クレート）の新設（後述。本 issue のスコープ外）。

## テスト

CLAUDE.md のテスト役割分担に従う。ただし本リポジトリには現状 PBT 基盤が無い（`pbt/` クレート・`proptest` 依存・`prop_*.rs` がいずれも存在せず、テストは `tests/*_tests.rs` の統合テストと各モジュールの `#[cfg(test)] mod tests` のみ）。よって本 issue の検証は既存のテスト機構で行い、PBT 基盤の新設は本 issue のスコープ外とする（必要なら別 issue で扱う）。

- 既存テストの更新（音声の型変更で壊れる箇所のみ）: `src/mp4/hybrid_writer.rs` の `#[cfg(test)] mod tests` の `make_audio_frame` ヘルパと音声側の `assert_eq!`（`last_audio_sample_entry` 比較は `.as_ref().map(|e| e.get())` 経由に直す）を更新する。`make_video_frame` と映像側の assert は据え置き。`tests/writer_mp4_tests.rs` の音声フレーム生成（`audio_data`）も `SharedSampleEntry::new(...)` でラップする。
- 単体テスト（新規）: 音声エンコーダが「最初の出力フレームだけでなく 2 フレーム目以降にも sample_entry を載せる」不変条件を検証する。これがバグ修正の核心の回帰防止。現状 `src/encoder/*.rs` には `#[cfg(test)] mod tests` が無いため、CLAUDE.md の命名規約（`tests/test_<module>.rs`）に従い `tests/test_encoder_opus.rs` 等を新設する。Opus は常時利用可能なので最低限 Opus で検証する。FDK-AAC（`fdk-aac` feature）と AudioToolbox（プラットフォーム依存）は feature / ターゲット有効時のみ走るよう `#[cfg(...)]` でガードする（無効時に検証できない点は許容する）。
- 単体テスト（新規）: `SharedSampleEntry::changed_since` の分岐（prev=None で true、同一 Arc で false、別 Arc・実体同値で false、別 Arc・実体相違で true）を検証する。
- 統合 / e2e: 下記「完了条件」の CI 反復で finalize 失敗の非再発を確認する。

## 完了条件

- 短時間 SRT 録画（`obsws/test_output.py::test_obsws_srt_inbound_with_stream_id` および `obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output`）を CI で十分な回数繰り返しても finalize 失敗が再発しないこと。発生率が約 3% のため、issues/0008 で用いた 100 回相当（10 シャード × 10 回）の一時ワークフローで検証し、検証後にそのワークフローは削除する。一時ワークフローの追加・削除は最終差分に残さない（同一ブランチ内で追加と削除を別コミットにし、マージ時に最終差分から消えることを確認する。具体的なワークフロー定義は issues/0008 のものを複製・改変する）。
- 検証には観測メトリクス（`hisui_total_finalize_failure_count` / `hisui_total_missing_audio_sample_entry_count` / `hisui_total_received_audio_sample_entry_count`）と warn ログ（`Missing sample entry for first sample of Audio track`）を使う。これらは feature/fix-hybrid-writer-finalize-on-stop-record で追加済み（`src/mp4/writer.rs` の `Mp4WriterStats`、`src/mp4/hybrid_writer.rs` の計上箇所）。
- `received_audio_sample_entry` カウンタの計上条件を定義し直す: このカウンタは現状 hybrid writer の入口（`src/mp4/hybrid_writer.rs:949-953`）でのみ計上され、通常の Mp4Writer 経路では常に 0（`src/mp4/writer.rs:80-89` のコメント参照）。全フレーム付与後に現状の `is_some()` 条件のまま入口で数えると、受信音声フレーム総数とほぼ一致し「上流が一度でも entry を送ったか」を表す意味が失われる。本 issue では計上条件を「入口で受信した sample_entry が直前の `last_audio_sample_entry` から変化したとき（`changed_since` が true）のみ」に変える。これは決定 3 の writer→muxer フィルタ適用の有無とは独立で、入口での計上ロジックの変更である。これにより `received` は「entry の確定・変化回数」を表し、finalize 失敗の主指標は `finalize_failure_count == 0` と `missing == 0`、`received` は補助指標とする。
- 上記のテスト（既存テストの更新・新規単体テスト）が通ること。
- `CHANGES.md` の `## develop` に `[FIX]` エントリを追記すること（録画映像トラック欠落のデータ破損修正、CLAUDE.md 変更履歴規約）。
- ブランチは `feature/fix-` で切る。フィールド型を `Option<SampleEntry>` → `Option<SharedSampleEntry>` に変えるが、これらは CLI バイナリ内部のフレーム型であり外部公開 API ではないため後方互換は壊れない（利用者から見た挙動変更は finalize 失敗の解消のみ）。よって `feature/change-` ではなく `feature/fix-` が妥当。
- 修正完了後に issues/0011 を close する（`git mv` で issues/closed へ移動し、issues/0011 に「## 解決方法」を追記する。CLAUDE.md の close 規約）。

## 関連

- issues/0011（reopen 済み。本 issue で真の修正を行う。完了時に本 issue が 0011 を close する）
- issues/0008（先行する flaky テストの issue。発生率 3% の出典。closed）
- issue 0027（映像側の sample_entry を `SharedSampleEntry` で全フレーム付与に統一するリファクタリング。本 issue 完了後に着手する）
- issue 0028（音声・映像の sample_entry フィールドを非 Option 化するリファクタリング。0017・0027 完了後に着手する）
