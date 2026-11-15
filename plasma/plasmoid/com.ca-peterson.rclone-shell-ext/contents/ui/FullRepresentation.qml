pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3
import org.kde.plasma.extras as PlasmaExtras
import org.kde.plasma.plasmoid

PlasmaExtras.Representation {
    id: full

    // The root PlasmoidItem (main.qml); provides status, call().
    property var host: null

    readonly property var st: host ? host.status : null
    readonly property bool available: st !== null
    property bool showAll: Plasmoid.configuration.showAllByDefault

    readonly property var recent: available && st.recent ? st.recent : []
    readonly property var transfers: available && st.transfers ? st.transfers : []
    readonly property var pins: available && st.offline && st.offline.enabled && st.offline.pins ? st.offline.pins : []
    readonly property bool offlineEnabled: available && st.offline && st.offline.enabled === true

    Layout.minimumWidth: Kirigami.Units.gridUnit * 18
    Layout.preferredWidth: Kirigami.Units.gridUnit * 22
    Layout.minimumHeight: Kirigami.Units.gridUnit * 14
    Layout.preferredHeight: Kirigami.Units.gridUnit * 26

    collapseMarginsHint: true

    function kindInfo(kind) {
        switch (kind) {
        case "Created": return { icon: "list-add-symbolic", text: i18n("New file") };
        case "Updated": return { icon: "document-edit-symbolic", text: i18n("Changed") };
        case "Deleted": return { icon: "edit-delete-symbolic", text: i18n("Deleted") };
        }
        return null;
    }

    function directionText(dir, kind) {
        if (dir === "Upload") {
            return kind === "Deleted" ? i18n("Deleted here, so removed from the cloud") : i18n("Uploaded to the cloud");
        }
        if (dir === "Download") {
            return kind === "Deleted" ? i18n("Deleted in the cloud, so removed here") : i18n("Downloaded from the cloud");
        }
        return "";
    }

    function dirName(path) {
        const i = path.lastIndexOf("/");
        return i > 0 ? path.substring(0, i) : "";
    }

    function baseName(path) {
        return path.substring(path.lastIndexOf("/") + 1);
    }

    function action(verb) {
        host.call("ServiceAction", [verb]);
    }

    header: PlasmaExtras.PlasmoidHeading {
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing
            Kirigami.Icon {
                source: full.host ? full.host.iconName : "rclone-shell-ext-loading"
                implicitWidth: Kirigami.Units.iconSizes.medium
                implicitHeight: Kirigami.Units.iconSizes.medium
            }
            Kirigami.Heading {
                level: 3
                Layout.fillWidth: true
                elide: Text.ElideRight
                text: full.available ? (full.st.healthLabel || "") : i18n("rclone UI")
            }
            PlasmaComponents3.Label {
                visible: full.available && (full.st.speedText || "") !== ""
                text: full.available ? (full.st.speedText || "") : ""
                opacity: 0.7
            }
        }
    }

    contentItem: Item {
        // Daemon missing.
        PlasmaExtras.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            visible: !full.available
            iconName: "rclone-shell-ext-loading"
            text: i18n("rclone UI service not running")
        }

        PlasmaComponents3.ScrollView {
            anchors.fill: parent
            visible: full.available
            contentWidth: availableWidth

            ColumnLayout {
                width: parent.width
                spacing: Kirigami.Units.smallSpacing

                // Warnings
                Repeater {
                    model: {
                        const w = [];
                        if (!full.available) return w;
                        if (full.st.outdatedRclone)
                            w.push({ type: Kirigami.MessageType.Warning, text: String(full.st.outdatedRclone) });
                        if (full.st.rcError)
                            w.push({ type: Kirigami.MessageType.Error, text: String(full.st.rcError) });
                        if (full.st.lastError)
                            w.push({ type: Kirigami.MessageType.Error, text: i18n("Last error: %1", String(full.st.lastError)) });
                        if (full.st.outOfSpace)
                            w.push({ type: Kirigami.MessageType.Warning, text: i18n("The cache disk is out of space. Free some space so uploads can finish.") });
                        return w;
                    }
                    delegate: Kirigami.InlineMessage {
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.margins: Kirigami.Units.smallSpacing
                        visible: true
                        type: modelData.type
                        text: modelData.text
                    }
                }

                // Active transfers
                Repeater {
                    model: full.transfers
                    delegate: RowLayout {
                        id: tr
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Kirigami.Icon {
                            source: tr.modelData.mimeIcon || "application-octet-stream"
                            implicitWidth: Kirigami.Units.iconSizes.medium
                            implicitHeight: Kirigami.Units.iconSizes.medium
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            PlasmaComponents3.Label {
                                Layout.fillWidth: true
                                elide: Text.ElideMiddle
                                text: tr.modelData.name
                            }
                            PlasmaComponents3.ProgressBar {
                                Layout.fillWidth: true
                                from: 0
                                to: 100
                                value: tr.modelData.percentage || 0
                            }
                        }
                        Kirigami.Icon {
                            source: tr.modelData.direction === "Upload" ? "go-up-symbolic" : "go-down-symbolic"
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                        }
                    }
                }

                // Offline folders
                Kirigami.Heading {
                    visible: full.offlineEnabled
                    level: 5
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    text: i18n("Offline folders")
                }
                PlasmaComponents3.Label {
                    visible: full.offlineEnabled && full.pins.length === 0
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    opacity: 0.7
                    text: i18n("Offline folders: finish setup in Settings")
                }
                Repeater {
                    model: full.offlineEnabled ? full.pins : []
                    delegate: RowLayout {
                        id: pin
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing
                        Kirigami.Icon {
                            source: pin.modelData.phaseIcon || "folder-symbolic"
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            PlasmaComponents3.Label {
                                Layout.fillWidth: true
                                elide: Text.ElideMiddle
                                text: pin.modelData.rel
                            }
                            PlasmaComponents3.Label {
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                                font: Kirigami.Theme.smallFont
                                opacity: 0.7
                                text: pin.modelData.error ? i18n("%1: %2", pin.modelData.phaseLabel, pin.modelData.error) : pin.modelData.phaseLabel
                                color: pin.modelData.error ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                            }
                        }
                    }
                }

                // Recent
                RowLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.rightMargin: Kirigami.Units.largeSpacing
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    Kirigami.Heading {
                        level: 5
                        Layout.fillWidth: true
                        text: i18n("Recent")
                    }
                    PlasmaComponents3.ToolButton {
                        visible: full.recent.length > 10 || full.showAll
                        text: full.showAll ? i18n("Show less") : i18n("Show all (%1)", full.recent.length)
                        onClicked: full.showAll = !full.showAll
                    }
                }
                PlasmaComponents3.Label {
                    visible: full.recent.length === 0
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    opacity: 0.7
                    text: i18n("Nothing synced since rclone started")
                }

                Repeater {
                    model: full.showAll ? full.recent.slice(0, 100) : full.recent.slice(0, 10)
                    delegate: PlasmaComponents3.ItemDelegate {
                        id: row
                        required property var modelData
                        readonly property var kind: full.kindInfo(modelData.kind)
                        Layout.fillWidth: true
                        onClicked: full.host.call("OpenDetails", [modelData.name])

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.smallSpacing
                            Kirigami.Icon {
                                source: row.modelData.mimeIcon || "application-octet-stream"
                                implicitWidth: Kirigami.Units.iconSizes.smallMedium
                                implicitHeight: Kirigami.Units.iconSizes.smallMedium
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                PlasmaComponents3.Label {
                                    Layout.fillWidth: true
                                    elide: Text.ElideMiddle
                                    text: full.baseName(row.modelData.name)
                                }
                                PlasmaComponents3.Label {
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                    font: Kirigami.Theme.smallFont
                                    opacity: 0.7
                                    color: row.modelData.error ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                                    text: {
                                        const parts = [];
                                        const d = full.dirName(row.modelData.name);
                                        if (d) parts.push(d);
                                        parts.push(row.modelData.error ? i18n("Failed: %1", row.modelData.error) : (row.modelData.atText || ""));
                                        if ((row.modelData.times || 1) > 1) parts.push("×" + row.modelData.times);
                                        return parts.join(" · ");
                                    }
                                }
                            }
                            Kirigami.Icon {
                                visible: row.modelData.direction !== null && row.modelData.direction !== undefined
                                source: row.modelData.direction === "Upload" ? "go-up-symbolic" : "go-down-symbolic"
                                implicitWidth: Kirigami.Units.iconSizes.small
                                implicitHeight: Kirigami.Units.iconSizes.small
                                HoverHandler { id: dirHover }
                                PlasmaComponents3.ToolTip {
                                    visible: dirHover.hovered
                                    text: full.directionText(row.modelData.direction, row.modelData.kind)
                                }
                            }
                            Kirigami.Icon {
                                visible: row.kind !== null
                                source: row.kind ? row.kind.icon : ""
                                implicitWidth: Kirigami.Units.iconSizes.small
                                implicitHeight: Kirigami.Units.iconSizes.small
                                HoverHandler { id: kindHover }
                                PlasmaComponents3.ToolTip {
                                    visible: kindHover.hovered && row.kind !== null
                                    text: row.kind ? row.kind.text : ""
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    footer: PlasmaExtras.PlasmoidHeading {
        position: PlasmaExtras.PlasmoidHeading.Footer
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing
            PlasmaComponents3.Button {
                icon.name: "folder-open-symbolic"
                text: i18n("Open folder")
                enabled: full.available
                onClicked: full.host.call("OpenFolder")
            }
            PlasmaComponents3.Button {
                icon.name: "view-refresh-symbolic"
                text: i18n("Restart")
                enabled: full.available
                onClicked: full.action("restart")
            }
            PlasmaComponents3.Button {
                readonly property bool active: !full.available || full.st.serviceActive
                icon.name: active ? "media-playback-stop-symbolic" : "media-playback-start-symbolic"
                text: active ? i18n("Stop") : i18n("Start")
                enabled: full.available
                onClicked: full.action(active ? "stop" : "start")
            }
            Item { Layout.fillWidth: true }
            PlasmaComponents3.Button {
                icon.name: "configure-symbolic"
                text: i18n("Settings…")
                enabled: full.available
                onClicked: full.host.call("OpenSettings")
            }
        }
    }
}
