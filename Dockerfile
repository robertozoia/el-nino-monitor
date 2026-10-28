# --- build -------------------------------------------------------------------
FROM rust:1-trixie AS build
WORKDIR /src

# Cache dependencies separately from the application code.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release \
 && rm -rf src target/release/deps/elnino* target/release/elnino*

COPY src src
COPY migrations migrations
# Compiled into the binary (include_bytes!/include_str! in src/sources/chirps.rs).
COPY static/geo static/geo
RUN cargo build --release

# --- runtime -----------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates sqlite3 tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /app elnino \
 && mkdir -p /data && chown elnino /data

WORKDIR /app
COPY --from=build /src/target/release/elnino /app/elnino
COPY templates templates
COPY i18n i18n
COPY static static

ENV DATA_DIR=/data \
    ASSETS_DIR=/app \
    BIND=0.0.0.0:8080 \
    RUST_LOG=info,tower_http=warn

USER elnino
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=60s --timeout=5s CMD ["/bin/bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q ok <&3"]
ENTRYPOINT ["/app/elnino"]
CMD ["serve"]
