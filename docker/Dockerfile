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
COPY release/ubuntu-${UBUNTU_VERSION}_x86_64/lyra/model_coeffs /tmp/amd64/lyra/model_coeffs
COPY release/ubuntu-${UBUNTU_VERSION}_arm64/lyra/model_coeffs /tmp/arm64/lyra/model_coeffs
RUN cp /tmp/hisui-${TARGETARCH} /usr/local/bin/hisui && mkdir -p /usr/local/lib && cp -r /tmp/${TARGETARCH}/lyra /usr/local/lib

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/hisui"]
