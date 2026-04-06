if [ -z "${XDG_CURRENT_DESKTOP}" ]; then
    export XDG_CURRENT_DESKTOP=AGL
fi

if [ -z "${XDG_RUNTIME_DIR}" ]; then
    export XDG_RUNTIME_DIR=/run/user/$(id -u)
fi

if [ -z "${DBUS_SESSION_BUS_ADDRESS}" ]; then
    export DBUS_SESSION_BUS_ADDRESS=unix:path=${XDG_RUNTIME_DIR}/bus
fi