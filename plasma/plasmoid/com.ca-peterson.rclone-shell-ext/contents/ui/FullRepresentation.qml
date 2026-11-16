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
    // The daemon's UI text (main.qml's `s`).
    property var s: ({})

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
                text: full.available ? (full.st.healthLabel || "") : full.s["panel-title"]
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
            text: full.s["panel-service-missing"]
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
                            w.push({ type: Kirigami.MessageType.Warning, text: String(full.st.outdatedText) });
                        if (full.st.rcError)
                            w.push({ type: Kirigami.MessageType.Error, text: String(full.st.rcError) });
                        if (full.st.lastError)
                            w.push({ type: Kirigami.MessageType.Error, text: String(full.st.lastErrorText) });
                        if (full.st.outOfSpace)
                            w.push({ type: Kirigami.MessageType.Warning, text: full.s["panel-out-of-space"] });
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
                                text: tr.modelData.line
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
                    text: full.s["offline-heading"]
                }
                PlasmaComponents3.Label {
                    visible: full.offlineEnabled
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    opacity: 0.7
                    text: full.available && full.st.offline ? String(full.st.offline.line) : ""
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
                                text: pin.modelData.detail
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
                        text: full.s["panel-recent-heading"]
                    }
                    PlasmaComponents3.ToolButton {
                        visible: full.recent.length > 10 || full.showAll
                        text: full.showAll ? full.s["panel-show-less"] : String(full.st.showAllText)
                        onClicked: full.showAll = !full.showAll
                    }
                }
                PlasmaComponents3.Label {
                    visible: full.recent.length === 0
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    opacity: 0.7
                    text: full.available ? String(full.st.emptyHistoryText) : ""
                }

                Repeater {
                    model: full.showAll ? full.recent.slice(0, 100) : full.recent.slice(0, 10)
                    delegate: PlasmaComponents3.ItemDelegate {
                        id: row
                        required property var modelData
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
                                    text: row.modelData.errorText ? row.modelData.errorText : row.modelData.caption
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
                                    text: row.modelData.directionTip || ""
                                }
                            }
                            Kirigami.Icon {
                                visible: !!row.modelData.kindIcon
                                source: row.modelData.kindIcon || ""
                                implicitWidth: Kirigami.Units.iconSizes.small
                                implicitHeight: Kirigami.Units.iconSizes.small
                                HoverHandler { id: kindHover }
                                PlasmaComponents3.ToolTip {
                                    visible: kindHover.hovered && !!row.modelData.kindIcon
                                    text: row.modelData.kindLabel || ""
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
                text: full.s["panel-open-folder"]
                enabled: full.available
                onClicked: full.host.call("OpenFolder")
            }
            PlasmaComponents3.Button {
                icon.name: "view-refresh-symbolic"
                text: full.s["panel-restart"]
                enabled: full.available
                onClicked: full.action("restart")
            }
            PlasmaComponents3.Button {
                readonly property bool active: !full.available || full.st.serviceActive
                icon.name: active ? "media-playback-stop-symbolic" : "media-playback-start-symbolic"
                text: active ? full.s["panel-stop"] : full.s["panel-start"]
                enabled: full.available
                onClicked: full.action(active ? "stop" : "start")
            }
            Item { Layout.fillWidth: true }
            PlasmaComponents3.Button {
                icon.name: "configure-symbolic"
                text: full.s["panel-settings"]
                enabled: full.available
                onClicked: full.host.call("OpenSettings")
            }
        }
    }
}
