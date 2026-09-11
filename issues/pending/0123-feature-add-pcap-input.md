# pcap 入力に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-pcap-input
- Polished:

## 目的

mp4 以外の入力ソースとして pcap ファイルからメディアを取り込めるようにする。顧客向けというより開発者向けの機能。

## pending とした理由

- pcap パーサと、対象とするプロトコル (RTP / RTMP / SRT など) の選定が必要。
- パケットの並び替え・欠落の扱いなど、復元処理の設計が必要。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/` に pcap 関連の実装は無い。
- ファイル入力は MP4 のみ (`ObswsInputSettings::Mp4FileSource` / `Mp4FileReader`)。WebM の読み込みは削除済み。
- その他の入力はネットワークプロトコルとデバイス・画像・色の source。

## 設計方針 (要検討・未確定)

- pcap を読み、対象プロトコルのパケットからメディアを復元する入力ソースを追加する。
- 対応プロトコルと、復元できなかった場合の扱いを決める。

## 完了条件

- pcap ファイルからメディアを取り込めること。

(解決方法は pending のため、設計確定後に記載する。)
