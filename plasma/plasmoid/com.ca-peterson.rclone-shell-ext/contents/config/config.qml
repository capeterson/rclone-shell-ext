import QtQuick
import org.kde.plasma.configuration
import org.kde.plasma.workspace.dbus as DBus

ConfigModel {
    id: model

    // The daemon's UI text; the Strings() call also starts the daemon if it isn't running.
    property var s: withFallback({})

    function withFallback(table) {
        return new Proxy(table, {
            get: function (target, id) {
                return target[id] !== undefined ? target[id] : String(id);
            }
        });
    }

    Component.onCompleted: {
        DBus.SessionBus.asyncCall({
            service: "com.ca-peterson.rclone-shell-ext",
            path: "/com/ca_peterson/rclone_shell_ext",
            iface: "com.ca_peterson.RcloneShellExt1",
            member: "Strings", arguments: []
        }, function (reply) {
            if (Array.isArray(reply)) reply = reply[0];
            if (reply !== null && typeof reply === "object" && reply.value !== undefined) reply = reply.value;
            if (reply !== null && typeof reply === "object") model.s = withFallback(reply);
        }, function (err) {});
    }

    ConfigCategory {
        name: model.s["settings-heading-general"]
        icon: "configure"
        source: "configGeneral.qml"
    }
}
