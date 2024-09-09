ARG UBUNTU_VERSION
FROM ubuntu:$UBUNTU_VERSION
LABEL maintainer="HARUYAMA Seigo <haruyama@tankyu.net>"

WORKDIR /usr/local/bin
RUN apt update \
    && apt install -y tini \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /var/cache/apt/archives/*

ARG UBUNTU_VERSION
ARG TARGETARCH
COPY release/ubuntu-${UBUNTU_VERSION}_x86_64/hisui /tmp/hisui-amd64
COPY release/ubuntu-${UBUNTU_VERSION}_arm64/hisui /tmp/hisui-arm64
RUN cp /tmp/hisui-$TARGETARCH /usr/local/bin/hisui

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/hisui"]
