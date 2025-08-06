# Ubuntu 24.04 をベースイメージとして使用
FROM ubuntu:24.04

# TARGETARCH を ARG として宣言
ARG TARGETARCH

# アーキテクチャに基づいて適切なバイナリをコピー
COPY hisui.${TARGETARCH} /usr/local/bin/hisui

# バイナリの実行権限を設定し、非 root ユーザーを作成
RUN chmod +x /usr/local/bin/hisui && \
    useradd -m -u 1000 hisui

# セキュリティのため非 root ユーザーで実行
USER hisui

# エントリーポイントを設定
ENTRYPOINT ["/usr/local/bin/hisui"]