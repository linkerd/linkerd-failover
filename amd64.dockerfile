ARG RUST_VERSION=1.63.0
ARG RUST_IMAGE=docker.io/library/rust:${RUST_VERSION}
ARG RUNTIME_IMAGE=gcr.io/distroless/cc

# Builds the operator binary.
FROM $RUST_IMAGE as build
ARG TARGETARCH
WORKDIR /build
COPY Cargo.toml Cargo.lock .
RUN mkdir -p ./cli/src && \
    echo 'fn main() {}' > ./cli/src/main.rs
COPY cli/Cargo.toml ./cli/Cargo.toml
COPY controller ./controller
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.63.0,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo fetch --locked
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.63.0,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo build --frozen --target=x86_64-unknown-linux-gnu --release --package=linkerd-failover-controller && \
    mv target/x86_64-unknown-linux-gnu/release/linkerd-failover-controller /tmp/

# Creates a minimal runtime image with the operator binary.
FROM $RUNTIME_IMAGE
COPY --from=build /tmp/linkerd-failover-controller /bin/
ENTRYPOINT ["/bin/linkerd-failover-controller"]
