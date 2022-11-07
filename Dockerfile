ARG DEV=ghcr.io/linkerd/dev:v33
FROM --platform=$BUILDPLATFORM $DEV-rust-cross as controller
WORKDIR /build
RUN mkdir -p target/bin
COPY Cargo.toml Cargo.lock .
RUN mkdir -p cli/src && \
    echo 'fn main() {}' > cli/src/main.rs
COPY cli/Cargo.toml cli/Cargo.toml
COPY controller controller
COPY justfile justfile
RUN --mount=type=cache,from=ghcr.io/linkerd/dev:v32-rust-cross,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo fetch --locked
ARG TARGETARCH
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=ghcr.io/linkerd/dev:v32-rust-cross,source=/usr/local/cargo,target=/usr/local/cargo \
    target=$(case "$TARGETARCH" in \
        amd64) echo x86_64-unknown-linux-musl ;; \
        arm64) echo aarch64-unknown-linux-musl ;; \
        arm) echo armv7-unknown-linux-musleabihf ;; \
        *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac) && \
    just target="$target" profile='release' controller-build && \
    mkdir /out && mv $(just --evaluate target="$target" profile='release' controller-bin) /out

FROM scratch as runtime
COPY --from=controller /out/linkerd-failover-controller /
ENTRYPOINT ["/linkerd-failover-controller"]
