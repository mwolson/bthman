FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
ENV container=docker

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        dbus \
        dbus-user-session \
        procps \
        systemd \
        systemd-sysv \
    && rm -rf /var/lib/apt/lists/*

# Remove unnecessary systemd units to speed up boot
RUN rm -f /lib/systemd/system/multi-user.target.wants/* \
    /etc/systemd/system/*.wants/* \
    /lib/systemd/system/local-fs.target.wants/* \
    /lib/systemd/system/sockets.target.wants/*udev* \
    /lib/systemd/system/sockets.target.wants/*initctl* \
    /lib/systemd/system/sysinit.target.wants/systemd-tmpfiles-setup* \
    /lib/systemd/system/systemd-update-utmp*

# Create non-root test user and enable lingering so systemd starts user@1000.service
RUN useradd -m -u 1000 testuser && \
    mkdir -p /var/lib/systemd/linger && touch /var/lib/systemd/linger/testuser

# Install the prebuilt bthman binary (produced by integration-tests/run.sh)
COPY integration-tests/dist/bthman /usr/local/bin/bthman
RUN chmod +x /usr/local/bin/bthman && \
    mkdir -p /home/testuser/.local/bin && \
    ln -sf /usr/local/bin/bthman /home/testuser/.local/bin/bthman && \
    chown -R testuser:testuser /home/testuser/.local

COPY systemd/ /build/systemd/
COPY openrc-system/ /build/openrc-system/
COPY openrc-user/ /build/openrc-user/

COPY integration-tests/shared/fake-pactl /usr/local/bin/pactl
COPY integration-tests/shared/fake-wpctl /usr/local/bin/wpctl
COPY integration-tests/shared/fake-bluetoothctl /usr/local/bin/bluetoothctl
COPY integration-tests/shared/fake-dbus-monitor /usr/local/bin/dbus-monitor
RUN chmod +x /usr/local/bin/pactl /usr/local/bin/wpctl /usr/local/bin/bluetoothctl /usr/local/bin/dbus-monitor

COPY integration-tests/systemd/test.sh /opt/test.sh
RUN chmod +x /opt/test.sh

STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
