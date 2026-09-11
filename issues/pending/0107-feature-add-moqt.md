# MOQT に対応する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-moqt
- Polished:

## 目的

Media over QUIC (MOQT) を hisui のメディア入出力として追加する。

## pending とした理由

- MOQT は仕様と実装が流動的で、HTTP/3 (QUIC) 基盤の選定と実装が必要。
- 対応するサブスクライブ/パブリッシュの範囲を決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/` に MOQT 関連の実装は無い。
- README の対応機能 (WebRTC / RTMP / SRT / RTSP / HLS / MPEG-DASH) にも含まれていない。

## 設計方針 (要検討・未確定)

- QUIC / HTTP/3 のライブラリを選定する。
- MOQT のサブスクライブ (入力) / パブリッシュ (出力) を `ObswsInputSettings` / `OutputKind` に追加する。
- メディア表現 (トラック・オブジェクト) の扱いを決める。

## 完了条件

- MOQT 経由のメディア入出力ができること。

(解決方法は pending のため、設計確定後に記載する。)
