# Hisui DevTools

[hisui](https://github.com/shiguredo/hisui) 向けの Web 開発ツール。
HTTP Bootstrap と DataChannel シグナリングによる P2P WebRTC 接続、OBS WebSocket over DataChannel のデバッグ UI を提供する。

公開版: https://hisui-devtools.shiguredo.app/

ライセンスはリポジトリルートの [Apache License 2.0](../LICENSE) に従う。

## 必要条件

- Node.js >= 24
- pnpm >= 11
- [Vite+](https://viteplus.dev/guide/) (`vp` コマンド)

## セットアップ

```bash
cd devtools
brew install pnpm   # 未インストールの場合
curl -fsSL https://vite.plus | bash
vp install
```

## 環境変数

`.env.template` をコピーして `.env` を作成する。

```bash
cp .env.template .env
```

| 変数                   | 説明                                                   |
| ---------------------- | ------------------------------------------------------ |
| `VITE_STUN_SERVER_URL` | STUN サーバーの URL (例: `stun:stun.example.com:3478`) |

## 使い方

### 開発サーバー

```bash
vp dev
```

デフォルトで `http://localhost:5173/` が開く。

### hisui server と組み合わせる

```bash
hisui server --ui --ui-remote-url http://localhost:5173/
```

hisui server の `--http-port` デフォルトは `4455` のため、Bootstrap URL は
`http://127.0.0.1:4455/bootstrap` になる。

## ページ

| パス             | 内容                                                       |
| ---------------- | ---------------------------------------------------------- |
| `/` (P2P)        | Bootstrap 接続、映像表示、OBS WebSocket (DataChannel) 操作 |
| `/debug` (Debug) | DataChannel 状態、Stats、OBS WebSocket リクエスト送信      |
