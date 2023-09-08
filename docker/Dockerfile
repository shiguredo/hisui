ARG UBUNTU_VERSION
FROM ubuntu:$UBUNTU_VERSION
LABEL maintainer="HARUYAMA Seigo <haruyama@tankyu.net>"

WORKDIR /usr/local/bin
RUN apt update \
    && apt install -y tini \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /var/cache/apt/archives/*

ENV HISUI_LYRA_MODEL_COEFFS_PATH /usr/local/lib/lyra/model_coeffs
ARG UBUNTU_VERSION
ARG TARGETARCH
COPY release/ubuntu-${UBUNTU_VERSION}_x86_64/hisui /tmp/hisui-amd64
COPY release/ubuntu-${UBUNTU_VERSION}_arm64/hisui /tmp/hisui-arm64
COPY third_party/lyra/lyra/lyra/model_coeffs /usr/local/lib/lyra/model_coeffs
RUN cp /tmp/hisui-$TARGETARCH /usr/local/bin/hisui

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/hisui"]
