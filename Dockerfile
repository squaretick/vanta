# Multi-stage build producing a small image with the `vanta` and `vanta-shim`
# binaries. Build: docker build -t vanta . — Run: docker run --rm vanta --version
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin vanta --bin vanta-shim

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/vanta /usr/local/bin/vanta
COPY --from=build /src/target/release/vanta-shim /usr/local/bin/vanta-shim
ENTRYPOINT ["vanta"]
CMD ["--help"]
