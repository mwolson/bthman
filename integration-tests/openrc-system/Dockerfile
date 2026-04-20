FROM alpine:3.21

RUN apk add --no-cache bash openrc procps

# Install the prebuilt bthman binary (produced by integration-tests/run.sh)
COPY integration-tests/dist/bthman /usr/local/bin/bthman
RUN chmod +x /usr/local/bin/bthman

COPY systemd/ /build/systemd/
COPY openrc-system/ /build/openrc-system/
COPY openrc-user/ /build/openrc-user/

COPY integration-tests/shared/fake-pactl /usr/local/bin/pactl
COPY integration-tests/shared/fake-wpctl /usr/local/bin/wpctl
COPY integration-tests/shared/fake-bluetoothctl /usr/local/bin/bluetoothctl
COPY integration-tests/shared/fake-dbus-monitor /usr/local/bin/dbus-monitor
RUN chmod +x /usr/local/bin/pactl /usr/local/bin/wpctl /usr/local/bin/bluetoothctl /usr/local/bin/dbus-monitor

COPY integration-tests/openrc-system/test.sh /opt/test.sh
RUN chmod +x /opt/test.sh

CMD ["/opt/test.sh"]
