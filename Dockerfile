FROM --platform=$BUILDPLATFORM ghcr.io/linkerd/dev:v32-rust-cross as build
WORKDIR /build
COPY Cargo.toml Cargo.lock .
RUN mkdir -p ./cli/src && \
    echo 'fn main() {}' > ./cli/src/main.rs
COPY cli/Cargo.toml ./cli/Cargo.toml
COPY controller ./controller
RUN --mount=type=cache,from=ghcr.io/linkerd/dev:v32-rust-cross,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo fetch --locked
ARG TARGETARCH
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=ghcr.io/linkerd/dev:v32-rust-cross,source=/usr/local/cargo,target=/usr/local/cargo \
     target=$(case "$TARGETARCH" in \
        amd64) echo x86_64-unknown-linux-gnu ;; \
        arm64) echo aarch64-unknown-linux-gnu ;; \
        arm) echo armv7-unknown-linux-gnueabihf ;; \
        *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac) && \
    cargo build --frozen --target="$target" --release --package=linkerd-failover-controller && \
    mv "target/$target/release/linkerd-failover-controller" /tmp/

FROM gcr.io/distroless/cc
COPY --from=build /tmp/linkerd-failover-controller /bin/
ENTRYPOINT ["/bin/linkerd-failover-controller"]
