#!/usr/bin/env bash
set -euo pipefail

DESTDIR="${DESTDIR:-}"
PREFIX="${PREFIX:-/usr}"
LIBEXECDIR="${LIBEXECDIR:-${PREFIX}/libexec}"
DATADIR="${DATADIR:-${PREFIX}/share}"
SYSCONFDIR="${SYSCONFDIR:-/etc}"
SYSTEMD_USER_DIR="${DATADIR}/systemd/user"

install -Dm755 target/release/xdg-desktop-portal-agl \
    "${DESTDIR}${LIBEXECDIR}/xdg-desktop-portal-agl"

install -Dm644 data/agl.portal \
    "${DESTDIR}${DATADIR}/xdg-desktop-portal/portals/agl.portal"

install -Dm644 data/agl-portals.conf \
    "${DESTDIR}${SYSCONFDIR}/xdg-desktop-portal/portals.conf"

install -Dm644 data/org.freedesktop.impl.portal.desktop.agl.service \
    "${DESTDIR}${DATADIR}/dbus-1/services/org.freedesktop.impl.portal.desktop.agl.service"

install -Dm644 data/xdg-desktop-portal-agl.service \
    "${DESTDIR}${SYSTEMD_USER_DIR}/xdg-desktop-portal-agl.service"

echo "Installed xdg-desktop-portal-agl to ${DESTDIR}${PREFIX}."