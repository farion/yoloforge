FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git ripgrep python3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 factory \
    && mkdir -p /home/factory/workspace \
    && chown -R factory:factory /home/factory
COPY --chmod=755 opencode /usr/local/bin/opencode
RUN opencode --version | grep -Fx 'opencode v2.0.27'
USER factory
ENV HOME=/home/factory
WORKDIR /home/factory/workspace
EXPOSE 4096
CMD ["opencode", "serve", "--hostname", "0.0.0.0", "--port", "4096"]
