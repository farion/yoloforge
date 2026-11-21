# Local acceptance fixture; the base is built by spikes/opencode/container/lab.py.
FROM yolofactory-opencode-spike:2.0.27
USER root
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/* \
    && opencode --version | grep -Fx 'opencode v2.0.27'
USER 10001:10001
