FROM alpine:edge

RUN apk add --no-cache bash openrc procps

# Create non-root test user
RUN adduser -D -u 1000 testuser

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

COPY integration-tests/openrc-user/test.sh /opt/test.sh
RUN chmod +x /opt/test.sh

CMD ["/opt/test.sh"]
