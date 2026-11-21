FROM rust:1.98-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY spikes/opencode/Cargo.toml ./spikes/opencode/Cargo.toml
COPY spikes/opencode/src ./spikes/opencode/src
RUN cargo build --locked --release -p factoryd

FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates curl python3 && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 factory && useradd --uid 10001 --gid factory --no-create-home factory \
    && install -d -o factory -g factory -m 0700 /data
COPY --from=build /src/target/release/factoryd /usr/local/bin/factoryd
COPY --chmod=0755 scripts/factory-backup.py /usr/local/bin/factory-backup
USER 10001:10001
WORKDIR /data
EXPOSE 7432
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 CMD curl --fail --silent http://127.0.0.1:7432/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/factoryd"]
CMD ["--listen","0.0.0.0:7432","--allow-container-network","--database","/data/factory.sqlite3"]
