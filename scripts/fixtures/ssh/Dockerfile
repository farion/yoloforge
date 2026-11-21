# Disposable SSH endpoint for smoke-ssh.py, never a production host configuration.
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /run/sshd /root/.ssh && chmod 700 /root/.ssh && passwd -d root
COPY scripts/fixtures/ssh/sshd_config /etc/ssh/sshd_config
CMD ["sh","-c","cp /keys/client.pub /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys && exec /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config"]
