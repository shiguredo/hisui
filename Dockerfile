# Ubuntu 24.04 をベースイメージとして使用
FROM ubuntu:24.04

# CI でビルドされたバイナリをコピー
COPY hisui /usr/local/bin/hisui
RUN chmod +x /usr/local/bin/hisui

# 実行ユーザーを作成
RUN useradd -m -u 1000 hisui
USER hisui

# エントリーポイントを設定
ENTRYPOINT ["/usr/local/bin/hisui"]