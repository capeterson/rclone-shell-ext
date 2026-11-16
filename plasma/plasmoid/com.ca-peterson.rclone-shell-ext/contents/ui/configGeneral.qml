import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM
import org.kde.plasma.workspace.dbus as DBus

KCM.SimpleKCM {
    id: page

    // Applet-local pref (cfg_ pattern); everything else talks to the daemon.
    property alias cfg_showAllByDefault: showAllBox.checked
    property bool cfg_showAllByDefaultDefault: false

    readonly property var fields: ["rc_addr", "env_file", "service", "mount_path", "cache_dir", "sync_interval_secs"]
    property var original: ({})
    // The daemon's own finished explanation of a failed read (may be empty), and whether the
    // read failed at all; the fallback wording "daemon not running" needs Strings().
    property string loadDetail: ""
    property bool loadFailed: false
    readonly property string loadError: loadFailed
        ? (loadDetail !== "" ? loadDetail : s["panel-service-missing"]) : ""
    property string saveError: ""
    property bool loaded: false

    // The daemon's UI text (id -> wording). A config page can open before the daemon runs; the
    // Strings() call starts it. Until it answers (stringsLoaded) there is no wording, so the
    // page shows a busy indicator; a timer retries if the daemon wasn't ready.
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
        bus("Strings", [], function (table) {
            if (table !== null && typeof table === "object") {
                page.s = withDefault(table);
                page.stringsLoaded = true;
            }
        });
    }

    function bus(member, args, ok, fail) {
        DBus.SessionBus.asyncCall({
            service: "com.ca-peterson.rclone-shell-ext",
            path: "/com/ca_peterson/rclone_shell_ext",
            iface: "com.ca_peterson.RcloneShellExt1",
            member: member, arguments: args || []
        }, function (reply) {
            if (Array.isArray(reply)) reply = reply[0];
            if (reply !== null && typeof reply === "object" && reply.value !== undefined) reply = reply.value;
            if (ok) ok(reply);
        }, function (err) {
            if (fail) fail(err);
        });
    }

    function load() {
        bus("GetConfig", [], function (json) {
            try {
                const cfg = JSON.parse(json);
                if (cfg.errorText !== undefined) {
                    // The daemon couldn't read the file and says why, already translated.
                    page.loadDetail = String(cfg.errorText);
                    page.loadFailed = true;
                    return;
                }
                page.original = cfg;
                rcAddr.text = cfg.rc_addr !== undefined ? String(cfg.rc_addr) : "";
                envFile.text = cfg.env_file !== undefined && cfg.env_file !== null ? String(cfg.env_file) : "";
                service.text = cfg.service !== undefined ? String(cfg.service) : "";
                mountPath.text = cfg.mount_path !== undefined ? String(cfg.mount_path) : "";
                cacheDir.text = cfg.cache_dir !== undefined ? String(cfg.cache_dir) : "";
                interval.value = cfg.sync_interval_secs !== undefined ? Number(cfg.sync_interval_secs) : 0;
                page.loaded = true;
                page.loadFailed = false;
                page.loadDetail = "";
            } catch (e) {
                console.warn("rclone UI: bad GetConfig reply:", e);
                page.loadDetail = "";
                page.loadFailed = true;
            }
        }, function (err) {
            page.loadDetail = "";
            page.loadFailed = true;
        });
    }

    function saveConfig() {
        if (!loaded) return;
        const current = {
            rc_addr: rcAddr.text, env_file: envFile.text, service: service.text,
            mount_path: mountPath.text, cache_dir: cacheDir.text,
            sync_interval_secs: interval.value
        };
        const changed = {};
        let any = false;
        for (const k of fields) {
            if (current[k] !== original[k] && String(current[k]) !== String(original[k] === null || original[k] === undefined ? "" : original[k])) {
                changed[k] = current[k];
                any = true;
            }
        }
        if (!any) return;
        bus("SetConfig", [JSON.stringify(changed)], function (err) {
            page.saveError = (typeof err === "string") ? err : "";
            if (page.saveError === "") {
                for (const k in changed) page.original[k] = changed[k];
            }
        }, function (err) {
            page.saveError = page.s["panel-service-missing"];
        });
    }

    Component.onCompleted: {
        loadStrings();
        load();
    }

    // Retry reading the wording until the daemon answers (it may still be starting).
    Timer {
        interval: 2000
        running: !page.stringsLoaded
        repeat: true
        onTriggered: page.loadStrings()
    }

    QQC2.BusyIndicator {
        anchors.centerIn: parent
        visible: !page.stringsLoaded
        running: visible
    }

    Kirigami.FormLayout {
        visible: page.stringsLoaded
        Kirigami.InlineMessage {
            Kirigami.FormData.isSection: false
            Layout.fillWidth: true
            visible: page.loadError !== ""
            type: Kirigami.MessageType.Error
            text: page.loadError
            actions: Kirigami.Action {
                text: page.s["plasma-config-retry"]
                icon.name: "view-refresh"
                onTriggered: page.load()
            }
        }
        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.saveError !== ""
            type: Kirigami.MessageType.Error
            text: page.saveError
        }

        QQC2.TextField {
            id: rcAddr
            Kirigami.FormData.label: page.s["plasma-config-rc-addr"]
            enabled: page.loaded
            Layout.fillWidth: true
        }
        QQC2.TextField {
            id: envFile
            Kirigami.FormData.label: page.s["plasma-config-env-file"]
            enabled: page.loaded
            Layout.fillWidth: true
        }
        QQC2.TextField {
            id: service
            Kirigami.FormData.label: page.s["plasma-config-service"]
            enabled: page.loaded
            Layout.fillWidth: true
        }
        QQC2.TextField {
            id: mountPath
            Kirigami.FormData.label: page.s["plasma-config-mount-path"]
            enabled: page.loaded
            Layout.fillWidth: true
        }
        QQC2.TextField {
            id: cacheDir
            Kirigami.FormData.label: page.s["plasma-config-cache-dir"]
            enabled: page.loaded
            Layout.fillWidth: true
        }
        QQC2.SpinBox {
            id: interval
            Kirigami.FormData.label: page.s["plasma-config-sync-interval"]
            enabled: page.loaded
            from: 0
            to: 86400
            editable: true
        }

        Item { Kirigami.FormData.isSection: true }

        QQC2.CheckBox {
            id: showAllBox
            Kirigami.FormData.label: page.s["plasma-config-popup"]
            text: page.s["plasma-config-show-all"]
        }
    }
}
