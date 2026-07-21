# hisui -x transcribe に MP4 字幕トラック出力を追加する

- Priority: Low
- Created: 2026-07-21
- Completed:
- Model: Opus 4.7
- Branch: feature/add-transcribe-mp4-subtitle-output
- Polished:

## 目的

`hisui -x transcribe` (issue 0063) が publish する `TextFrame` を MP4 字幕トラックに muxing して書き出す機能を追加する。 標準出力への JSON LINE 出力に加え、字幕トラック付き MP4 を生成する経路を用意し、モダンプレイヤー (Web / DASH / QuickTime / VLC 等) で字幕付き再生できるようにする。

## 優先度根拠

Low。 mp4-rs 側の字幕対応 (0042-0046) の完了待ちで、いま緊急に対応する必要はない。 機械処理用途は 0063 の JSON LINE 出力でカバー済みの想定。 本 issue はプレイヤー字幕再生の付加価値の位置づけ。

## 現状

- hisui: 0063 で追加予定の `hisui -x transcribe` は標準出力への JSON LINE 出力のみ。 0063 の設計方針 (`issues/0063-feature-add-transcribe-subcommand.md`) には「SRT / WebVTT / データチャネル等の本格的な外部出力経路は 0014 範囲で別 issue で扱う」と明記されている。
- hisui: `MediaFrame::Text` / `TextFrame` (`src/text.rs`) は 0060 で追加済み。 `TranscriptionProcessor` (`src/ml/audio/transcription_processor.rs`) が 0062 で publish 可能な状態にある。 出力側の text subscriber は 0063 で JSON LINE 用のものが用意される想定。
- mp4-rs: 字幕トラック共通基盤 + 3 方式 + Mp4FileMuxer 字幕受け入れが以下の 5 issue で進行中 (いずれも open / Priority Low)。
  - `shiguredo/mp4-rs` 0042 字幕トラック共通基盤 (`TrackKind::Subtitle` / handler type / `MediaHeader` / `Fmp4SegmentMuxer` の字幕受け入れ、`Mp4FileMuxer` は `UnsupportedTrackKind` で明示拒否)。 Polished: 2026-07-21
  - `shiguredo/mp4-rs` 0043 stpp (ISO/IEC 14496-30 XMLSubtitleSampleEntry)
  - `shiguredo/mp4-rs` 0044 wvtt (ISO/IEC 14496-30 WVTTSampleEntry)
  - `shiguredo/mp4-rs` 0045 tx3g (3GPP TS 26.245)
  - `shiguredo/mp4-rs` 0046 `Mp4FileMuxer` での字幕トラック受け入れ

## 依存関係

以下の完了に依存する。

- hisui 0063: `hisui -x transcribe` サブコマンドが実装され、`TranscriptionProcessor` から `TextFrame` を publish する経路が動作している状態
- mp4-rs 0042 (共通基盤): `TrackKind::Subtitle` / `SthdBox` / `NmhdBox` / `MediaHeader` / `Fmp4SegmentMuxer` の字幕受け入れが揃う
- mp4-rs 0044 (wvtt): 本 issue で採用予定の字幕方式 (`WVTTSampleEntry` + `vttc` サンプル)
- mp4-rs 0046 (`Mp4FileMuxer` 字幕): 単一 MP4 ファイル出力を採る場合に必須 (`Fmp4SegmentMuxer` 出力にする選択肢もあるが、標準的な単一ファイル出力の期待に合わせる想定)

mp4-rs 0043 (stpp) / 0045 (tx3g) は本 issue のスコープ外。 hisui の音声認識由来の平文字幕には WVTT が最適な粒度で、複数方式を並存させる必要性は現時点では無いと判断する。

## 設計方針

### 出力先の口

- `hisui -x transcribe` に `--output <path>` オプションを追加する。 拡張子で出力形式を判別する (`.jsonl` / `.mp4`)
- 未指定時は既存の標準出力 JSON LINE を維持する (0063 の挙動を後方互換で保つ)
- `--format <jsonl|mp4>` を併設して拡張子と衝突した場合にどちらを優先するか、あるいは拡張子判別だけにするかは polish 時に確定する

### 字幕方式: WVTT (WebVTT) を採用

理由:

- モダンプレイヤー (Chrome / Safari / DASH.js / VLC / QuickTime) の対応幅が広い
- WebVTT のテキスト表現は Whisper 出力 (プレーンテキスト、時刻付き) にそのままマップできる
- stpp (XML) は表現力が高いが、平文字幕のユースケースでは過剰
- tx3g は QuickTime 系レガシー主体で採用範囲が狭く、Web / DASH 系で互換性が劣る

### TextFrame → 字幕サンプルの対応

- `TextFrame.start` / `end`: 字幕サンプルの presentation time と持続時間
- `TextFrame.text`: WebVTT `cue` の payload (プレーンテキスト、改行なし想定)
- `TextFrame.language`: `tkhd` / `mdhd` の language フィールド (ISO 639-2 の 3 文字 packed 表現、Whisper 言語コード ISO 639-1 → 639-2 変換のマッピング表を実装する)
- `TextFrame.no_speech_prob` / `avg_logprob`: 字幕サンプルには載せない (JSON LINE 出力側のみ)

### 内部実装の流れ

```
subcommand_transcribe::try_run (--output <path>.mp4 指定時)
  ├ MediaPipeline 組み立て
  ├ source processor (WAV/MP4 → MediaFrame::Audio publish、0063 で実装済み)
  ├ TranscriptionProcessor (0062 で実装済み)
  └ mp4 subtitle sink processor
        ├ MediaFrame::Text subscribe
        ├ mp4-rs の Mp4FileMuxer (0046 完了後) または Fmp4SegmentMuxer で
        │  字幕トラック付き MP4 を書き出す
        └ EOS でファイルを close
```

### polish 時に確定させる項目

- `--format` を併設するか、拡張子判別だけにするか
- 入力が MP4 (映像あり) の場合、生成される MP4 に元映像トラックをコピーするか、字幕トラック単独の MP4 にするか
- 音声のみ入力 (WAV / 音声のみ MP4) で字幕トラック単独の MP4 を生成する場合の制約 (mp4-rs 0046 の完了状況 + `Mp4FileMuxer` の要件次第)
- e2e テストの検証手段 (`hisui inspect` 相当 / mp4-rs API 直接 / ffprobe のいずれで assert するか)
- 生成される MP4 の compatible brand (`msubs` 等) の付与要否

### CHANGES.md エントリ

- `[ADD] hisui -x transcribe に MP4 字幕トラック (WVTT) 出力を追加する`
- 担当者: @sile

### e2e テスト

- `e2e-tests/` (pytest) で `hisui -x transcribe --output <tmp>.mp4 --language en <input>` を実行し、生成された MP4 が以下を満たすことを確認する
  - 字幕トラック (handler type `text`、SampleEntry `wvtt`) を含む
  - サンプル数が非零で、各サンプルの持続時間が入力音声の長さと矛盾しない
  - `tkhd` / `mdhd` の language フィールドが指定言語 (`en` → ISO 639-2 `eng`) と一致する
  - テキスト内容が期待言語の表記種類 (英語 fixture は英字、日本語 fixture は日本語文字) を含む

## 完了条件

- `hisui -x transcribe --output <path>.mp4` が動作し、WVTT 字幕トラック付き MP4 が生成される
- 生成された MP4 が Web ブラウザ (Chrome / Safari) と VLC で字幕表示できる (手動確認 + e2e テストで自動確認)
- `--output` 未指定時は既存の JSON LINE 出力を維持し、後方互換を破らない
- `docs/command_transcribe.md` (0063 で作成予定) に `--output <path>.mp4` の使い方を追記する
- CHANGES.md に `[ADD] hisui -x transcribe に MP4 字幕トラック (WVTT) 出力を追加する` エントリが追記されている
- `cargo test --features candle -p hisui` が test-candle CI job で green
- e2e テストが green (CI で実推論 + 実 muxing)

## 解決方法

0063 で実装される `subcommand_transcribe` の text subscriber を拡張し、`--output` 拡張子が `.mp4` の場合は mp4-rs (0042 + 0044 + 0046 完了後) の `Mp4FileMuxer` に `TextFrame` を字幕トラックサンプルとして流すヘルパを実装する。 サンプルエントリは WVTT。 現時点では mp4-rs 側の依存 issue が未完了のため、本 issue の実装着手は mp4-rs のリリース (0042 + 0044 + 0046 を含むバージョン) 待ち。
