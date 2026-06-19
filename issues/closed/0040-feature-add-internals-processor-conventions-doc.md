# プロセッサの種類別の実装規約と不変条件を docs/internals/ にまとめる

- Priority: Low
- Created: 2026-06-17
- Completed: 2026-06-19
- Model: Claude Opus 4.7
- Branch: feature/add-internals-processor-conventions-doc
- Polished:

## 目的

Hisui の各プロセッサ（入力プロセッサ / 出力プロセッサ / エンコーダ / デコーダ / mixer 等）が守るべき実装規約・不変条件を、種類別に整理した統一ドキュメントを `docs/internals/` 配下に追加する。

現状これらの規約は次の 2 箇所に分散しており、新規プロセッサ追加時に「どの規約を守るべきか」を一発で参照できる集約ドキュメントが無い:

- 過去の closed issue（0017 / 0027 / 0030 / 0031 / 0032 / 0033 / 0034 など）の本文・解決方法
- `src/audio.rs:87-93` / `src/video.rs:51-57` / `src/sample_entry.rs` 等の docstring

issue 0031 が WebM リーダーで「圧縮フレームは常に sample_entry を持つ」不変条件違反を解消する作業だったように、暗黙の規約に対する違反が混入しうる状況になっている。

## 優先度根拠

Low。既存実装は規約を満たしており、ドキュメント不在による直接の実害は今のところ無い。ただし新規プロセッサ追加時に規約の見落としが起こりうるため、整理する価値はある。

## 現状

`docs/internals/` の既存ドキュメント:

- `README.md`
- `architecture_overview.md`
- `bootstrap.md`
- `media_pipeline.md`
- `mixer.md`
- `obsws.md`
- `processor_id.md`
- `stats.md`
- `timestamp.md`

これらは個別のサブシステム解説で、「プロセッサ種別ごとに守るべき実装規約」の観点では整理されていない。特に `media_pipeline.md` はパイプライン全体の流れを説明しており、本 issue で追加するドキュメントの上位概念になり得る。

`src/` 配下のプロセッサ実装:

- 入力系: `src/webm/` / `src/mp4/` / `src/rtsp/` / `src/srt/` / `src/rtmp/` / `src/sora/` / `src/webrtc/`
- 出力系: `src/mp4/writer.rs` / `src/mp4/hybrid_writer.rs` / `src/dash/writer.rs` / `src/hls/writer.rs` / `src/sora_publisher.rs` 等
- 変換系: `src/encoder/` / `src/decoder/`
- 集約系: `src/mixer/`
- 共通型: `src/audio.rs` / `src/video.rs` / `src/sample_entry.rs`

## 検討事項

検討事項が複数あるため、本 issue は起票時点で内容を未確定のまま残し、polish-issue で順次詰める。以下を polish 時に確定させる:

### 1. プロセッサの分類軸

候補:

- 入力プロセッサ（リーダー / サブスクライバ / 録画 source）
- 出力プロセッサ（writer / publisher）
- 変換プロセッサ（エンコーダ / デコーダ）
- 集約プロセッサ（mixer）
- 制御プロセッサ（coordinator / dispatcher 等。obsws 配下を含むか）

これらが網羅的か / 重複しないかを判定する。

### 2. 各分類で整理すべき不変条件・規約の候補

- sample_entry 付与（入力プロセッサが圧縮フレームに `Some(SharedSampleEntry)` を必須にする等。issue 0017 / 0027 / 0030 / 0031 / 0032 / 0033 由来）
- タイムスタンプの単調性・時刻空間（`src/timestamp.md` との分担）
- チャネル数 / サンプルレートの前提（Hisui 固定値があるか）
- keyframe 要件（録画開始時のキーフレーム要求等）
- エラー伝播の扱い（fail-fast / 警告ログ + 続行 / 構造的に発生しない異常）
- フレーム生成 / 消費の単位（フレーム単位 / バーストモード）
- 統計フィールドの命名規則と `stats.md` との関係
- writer 入口の不変条件違反検知（issue 0034）の扱い

### 3. 既存ドキュメントとの分担と内部リンク方針

- `media_pipeline.md` との関係（上位概念か並列か）
- `processor_id.md` / `stats.md` / `timestamp.md` との内部リンク
- 既存ドキュメントに追記するか新規ファイルを起こすか

### 4. ドキュメント構成

候補:

- プロセッサ種別ごとに 1 ファイル（例: `processor_input.md` / `processor_output.md` / `processor_encoder.md` / `processor_decoder.md` / `processor_mixer.md`）
- 統合ファイル 1 つ（例: `processor_conventions.md`）
- 既存 `media_pipeline.md` を拡張する形

### 5. 規約の維持運用

新規 closed issue で規約が拡張・変更されたとき、ドキュメントを追従させる仕組み（CHANGES.md 起源か docstring 起源か、各 issue の完了条件にドキュメント更新を含めるか）。

## 設計方針

未確定。上記検討事項を polish-issue で詰めてから記載する。

## 完了条件

- `docs/internals/` 配下にプロセッサ種類別の規約ドキュメントが追加されていること
- 既存の不変条件・規約（issue 0017 / 0027 / 0030 / 0031 / 0032 / 0033 / 0034 由来）が網羅されていること
- 既存 `docs/internals/` ドキュメント（特に `media_pipeline.md` / `processor_id.md`）との内部リンクが整備されていること

### CHANGES.md

記載するかは未確定（内部ドキュメント追加で公開 API・利用者挙動の変化は無いため、未記載が妥当な可能性が高い）。polish-issue で確定する。

## 関連

- issue 0017 / 0027 / 0030 / 0031（音声 / 映像 sample_entry の全フレーム付与不変条件の起源）
- issue 0032 / 0033（RTSP / SRT Annex-B 映像経路への不変条件拡張）
- issue 0034（writer 入口の不変条件違反検知）
- issue 0039（writer 側 fallback 補完経路の削除可能性調査。本 issue が前提となる規約ドキュメントを定義する位置づけ）

## クローズ理由（2026-06-19・既存 docs/internals/ と他 open issue で実質網羅されるため実装せず close）

本 issue が想定した「プロセッサ種別ごとの規約・不変条件まとめ」の規約候補 8 項目を `docs/internals/` の既存ドキュメントおよび open issue（0039 / 0046）の射程に照合した結果、独立した統一ドキュメントを起こす意義が小さいと判断した。

### 既存 docs/internals/ で網羅済み

- timestamp の単調性・時刻空間: `timestamp.md`（`TimestampMapper` / `WrappingTimestampNormalizer` / `SampleBasedTimestampAligner`）
- channels / sample_rate の前提: `mixer.md` の「sample rate / channels の統一」節
- フレーム生成 / 消費の単位: `media_pipeline.md`（`Message::Media` / `Eos` / `Syn` の 3 種）、`mixer.md`（audio / video 比較表）
- 統計フィールドの命名規則: `stats.md` および `processor_id.md`（カテゴリ命名規則表）

### 他 open issue で吸収予定

- validation 責務分担: issue 0046（5 processor 構造体の validation 責務分担を確定する）の設計方針節が「`docs/internals/` 配下に責務分担ノートとして残す（0040 との合流候補）」を明示。0046 完了時にその章として `docs/internals/` 配下に生まれる
- sample_entry 不変条件・writer 入口違反検知: issue 0039（writer 側 fallback 補完経路の削除可能性調査）の結論次第で writer 入口違反検知節の要否が変わるため、集約は 0039 完了後に判断する

### 単独 issue で起こすには分量不足

keyframe 要件・エラー伝播の規約は確かに既存 docs に集約されていないが、新規 processor 追加で痛みが具体化したタイミングで既存 `docs/internals/` 配下の該当ファイルに追記する incremental approach のほうが「If it hurts, do it more often」原則と整合する。

### issue 0039 への影響

issue 0039 の関連節は「本 issue（0040）が前提となる規約ドキュメントを定義する位置づけ」と書いているが、0040 の規約ドキュメントは不要と判断したため、0039 はこの前提なしで進めて問題ない（0039 自体の調査範囲は writer 側 fallback の削除可否で完結する）。
