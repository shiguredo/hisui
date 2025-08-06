# Ubuntu 24.04 をベースイメージとして使用
FROM ubuntu:24.04

# CI でビルドされたバイナリをコピー
COPY hisui /usr/local/bin/hisui
RUN chmod +x /usr/local/bin/hisui

# セキュリティのため非 root ユーザーで実行
# UID 1000 は一般的なユーザー ID で、ホストとの権限マッピングに便利
RUN useradd -m -u 1000 hisui
USER hisui

# エントリーポイントを設定
ENTRYPOINT ["/usr/local/bin/hisui"]