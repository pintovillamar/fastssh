# syntax=docker/dockerfile:1

# Where the fastssh binary comes from:
#   build     compile it in this image (the default; `docker build .` just works)
#   prebuilt  take dist/<arch>/fastssh from the build context (what the
#             release workflow does, so nothing is compiled twice)
ARG BINARY=build

FROM node:24-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1-slim-trixie AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake perl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=web /src/web/dist ./web/dist
RUN cargo build --release --locked && cp target/release/fastssh /fastssh

FROM scratch AS prebuilt
ARG TARGETARCH
COPY dist/${TARGETARCH}/fastssh /fastssh

FROM ${BINARY} AS binary

FROM debian:trixie-slim
# ca-certificates: Google sign-in talks to Google over https.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data --create-home fastssh
COPY --from=binary --chmod=755 /fastssh /usr/local/bin/fastssh
# A shell inside this container would be of no use, so the local shell is off.
ENV FASTSSH_LISTEN=0.0.0.0:7422 \
    FASTSSH_DATA_DIR=/data \
    FASTSSH_NO_LOCAL_SHELL=true
USER fastssh
VOLUME /data
EXPOSE 7422
ENTRYPOINT ["fastssh"]
