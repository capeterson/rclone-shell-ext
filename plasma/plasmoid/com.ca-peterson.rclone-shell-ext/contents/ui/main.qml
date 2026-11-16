import QtQuick
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.workspace.dbus as DBus

PlasmoidItem {
    id: root

    readonly property string busService: "com.ca-peterson.rclone-shell-ext"
    readonly property string busPath: "/com/ca_peterson/rclone_shell_ext"
    readonly property string busIface: "com.ca_peterson.RcloneShellExt1"

    // The daemon's UI text (id -> wording), from Strings(). Until it has been read (see
    // stringsLoaded) every id is empty, and the popup shows a busy indicator instead.
    property var s: withDefault({})
    property bool stringsLoaded: false

    // Unknown ids read as "" so a binding never assigns undefined to a text property.
    function withDefault(table) {
        return new Proxy(table, {
            get: function (target, id) {
                return target[id] !== undefined ? target[id] : "";
            }
        });
    }

    function loadStrings() {
        call("Strings", [], function (table) {
            if (table !== null && typeof table === "object") {
                s = withDefault(table);
                stringsLoaded = true;
            }
        });
    }

    // Parsed StatusJson, or null when the daemon is absent / unparsable.
    property var status: null
    property string health: ""
    readonly property bool available: status !== null

    // The daemon names the icon for the current health; until it answers, the applet's own icon.
    readonly property string iconName: (available && status.healthIcon) ? status.healthIcon : Plasmoid.metaData.iconName

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
        // Calling the daemon also starts it; its text is read once it answers.
        if (!stringsLoaded) loadStrings();
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

    toolTipMainText: available ? (status.healthLabel || s["panel-title"]) : s["panel-service-missing"]
    toolTipSubText: available ? (status.speedText || "") : ""

    // Plasma's default compact representation shows Plasmoid.icon (which follows the health)
    // and handles click-to-expand the way panels expect.
    preferredRepresentation: compactRepresentation
    fullRepresentation: FullRepresentation {
        host: root
        s: root.s
        stringsLoaded: root.stringsLoaded
    }
}
