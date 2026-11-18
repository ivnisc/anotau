FROM rust:1.88.0-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY src ./src
COPY assets ./assets
COPY server ./server
COPY protocol ./protocol
RUN cargo build --release --locked -p anotau-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends openssl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin anotau \
    && install -d -m 0700 -o anotau -g anotau /data
COPY --from=build /build/target/release/anotau-server /usr/local/bin/anotau-server
USER anotau
CMD ["anotau-server"]
