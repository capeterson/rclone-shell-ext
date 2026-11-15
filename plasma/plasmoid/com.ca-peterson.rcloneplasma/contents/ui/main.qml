import QtQuick
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.workspace.dbus as DBus

PlasmoidItem {
    id: root

    readonly property string busService: "com.ca-peterson.RclonePlasma"
    readonly property string busPath: "/com/ca_peterson/RclonePlasma"
    readonly property string busIface: "com.ca_peterson.RclonePlasma1"

    // Parsed StatusJson, or null when the daemon is absent / unparsable.
    property var status: null
    property string health: ""
    readonly property bool available: status !== null

    readonly property var iconNames: ({
        "ServiceStopped": "stopped", "NotMounted": "unmounted", "Unreachable": "unreachable",
        "CacheFull": "full", "FileErrors": "error", "Uploading": "uploading",
        "Downloading": "downloading", "Transferring": "transferring",
        "Healthy": "synced", "Loading": "loading"
    })
    readonly property string iconName: "rclone-shell-ext-" + (available ? (iconNames[health] || "loading") : "loading")

    function unwrap(v) {
        if (Array.isArray(v)) v = v[0];
        if (v !== null && typeof v === "object" && v.value !== undefined) v = v.value;
        return v;
    }

    // Generic call on the daemon interface.
    function call(member, args, onOk, onFail) {
        DBus.SessionBus.asyncCall({
            service: busService, path: busPath, iface: busIface,
            member: member, arguments: args || []
        }, function (reply) {
            if (onOk) onOk(unwrap(reply));
        }, function (err) {
            if (onFail) onFail(err);
        });
    }

    function getProperty(name, onOk, onFail) {
        DBus.SessionBus.asyncCall({
            service: busService, path: busPath,
            iface: "org.freedesktop.DBus.Properties", member: "Get",
            arguments: [busIface, name]
        }, function (reply) {
            onOk(unwrap(reply));
        }, function (err) {
            if (onFail) onFail(err);
        });
    }

    function markAbsent() {
        status = null;
        health = "";
    }

    function refresh() {
        getProperty("StatusJson", function (json) {
            try {
                const parsed = JSON.parse(json);
                status = parsed;
                health = parsed.health || "";
            } catch (e) {
                console.warn("rclone UI: bad StatusJson:", e);
                markAbsent();
            }
        }, markAbsent);
        getProperty("Health", function (h) {
            if (typeof h === "string" && h !== "") health = h;
        });
    }

    // PropertiesChanged subscription is not wired up; polling is enough.
    Timer {
        interval: 2000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    Plasmoid.icon: iconName
    Plasmoid.status: (available && health === "Healthy")
        ? PlasmaCore.Types.PassiveStatus : PlasmaCore.Types.ActiveStatus

    toolTipMainText: available ? (status.healthLabel || i18n("rclone UI")) : i18n("rclone UI service not running")
    toolTipSubText: available ? (status.speedText || "") : ""

    // Plasma's default compact representation shows Plasmoid.icon (which follows the health)
    // and handles click-to-expand the way panels expect.
    preferredRepresentation: compactRepresentation
    fullRepresentation: FullRepresentation {
        host: root
    }
}
