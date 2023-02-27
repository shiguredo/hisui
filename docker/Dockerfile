FROM ubuntu:20.04
LABEL maintainer="HARUYAMA Seigo <haruyama@tankyu.net>"

WORKDIR /usr/local/bin
RUN apt update \
    && apt install -y tini \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /var/cache/apt/archives/*

ARG TARGETARCH
COPY release/ubuntu-20.04_x86_64/hisui /tmp/hisui-amd64
COPY release/ubuntu-20.04_arm64/hisui /tmp/hisui-arm64
RUN cp /tmp/hisui-$TARGETARCH /usr/local/bin/hisui

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/hisui"]
