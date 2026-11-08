# The losscape server and its web UI, in one small image. Build from the repository root:
#   docker build -f deploy/Dockerfile -t losscape .

# ---- The web UI: a static site ------------------------------------------------------
FROM node:24-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---- The server --------------------------------------------------------------------
FROM rust:1.99-bookworm AS server
RUN apt-get update && apt-get install -y --no-install-recommends python3
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY proto/ proto/
COPY scripts/third_party_notices.py scripts/
RUN cargo build --release --locked -p losscape-server \
    && mkdir -p /out \
    && cp target/release/losscape-server /out/ \
    && python3 scripts/third_party_notices.py --package losscape-server \
        --output /out/THIRD-PARTY-NOTICES-SERVER.txt --json /out/third-party-server.json

# ---- What runs: no shell, no package manager, not root ------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=server /out/losscape-server /usr/local/bin/losscape-server
COPY LICENSE /app/LICENSE
COPY --from=web /src/web/build /app/web
# Served with the web interface, for its About page.
COPY --from=server /out/THIRD-PARTY-NOTICES-SERVER.txt /out/third-party-server.json /app/web/
ENV LOSSCAPE_WEB_DIR=/app/web
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=5s --start-period=30s --retries=3 \
    CMD ["losscape-server", "healthcheck"]
ENTRYPOINT ["losscape-server"]
