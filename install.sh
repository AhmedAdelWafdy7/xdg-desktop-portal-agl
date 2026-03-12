#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr}"
LIBEXECDIR="${LIBEXECDIR:-${PREFIX}/libexec}"
DATADIR="${DATADIR:-${PREFIX}/share}"
SYSTEMD_USER_DIR="${DATADIR}/systemd/user" 

install -Dm755 target/release/xdg-desktop-portal-agl \
    "${LIBEXECDIR}/xdg-desktop-portal-agl"

install -Dm644 data/agl.portal \
    "${DATADIR}/xdg-desktop-portal/portals/agl.portal"

install -Dm644 data/agl-portals.conf \
    "${DATADIR}/xdg-desktop-portal/agl-portals.conf"

install -Dm644 data/org.freedesktop.impl.portal.desktop.agl.service \
    "${DATADIR}/dbus-1/services/org.freedesktop.impl.portal.desktop.agl.service"

install -Dm644 \
    "target/release/build/xdg-desktop-portal-agl-*/out/xdg-desktop-portal-agl.service" \
    "${SYSTEMD_USER_DIR}/xdg-desktop-portal-agl.service"

echo "Installed xdg-desktop-portal-agl to ${PREFIX}."