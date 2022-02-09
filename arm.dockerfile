ARG RUST_VERSION=1.58.1
ARG RUST_IMAGE=docker.io/library/rust:${RUST_VERSION}
ARG RUNTIME_IMAGE=gcr.io/distroless/cc

# Builds the operator binary.
FROM $RUST_IMAGE as build
RUN apt-get update && \
    apt-get install -y --no-install-recommends g++-arm-linux-gnueabihf libc6-dev-armhf-cross && \
    apt-get clean && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/ && \
    rustup target add armv7-unknown-linux-gnueabihf
ENV CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc
WORKDIR /build
COPY Cargo.toml Cargo.lock . /build/
# XXX(ver) we can't easily cross-compile against openssl, so use rustls on arm.
RUN --mount=type=cache,target=target \
    --mount=type=cache,from=rust:1.56.1,source=/usr/local/cargo,target=/usr/local/cargo \
    cargo build --locked --release --target=armv7-unknown-linux-gnueabihf \
        --package=linkerd-failover --no-default-features --features="rustls" && \
    mv target/armv7-unknown-linux-gnueabihf/release/linkerd-failover /tmp/

# Creates a minimal runtime image with the operator binary.
FROM --platform=linux/arm $RUNTIME_IMAGE
COPY --from=build /tmp/linkerd-failover /bin/
ENTRYPOINT ["/bin/linkerd-failover"]
