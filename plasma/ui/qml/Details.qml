// SPDX-License-Identifier: GPL-3.0-only
// The details window for one file or folder (`rclone-plasma --menu PATH`). A fixed-size window:
// one loading screen until everything has arrived, then the finished layout in one step.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import com.ca_peterson.RclonePlasma

Kirigami.ApplicationWindow {
    id: root

    title: "rclone"
    width: 480
    height: 680
    minimumWidth: width
    maximumWidth: width
    minimumHeight: height
    maximumHeight: height

    DetailsModel {
        id: details
        Component.onCompleted: details.load([])
    }

    readonly property var permissions: {
        try {
            return JSON.parse(details.permissionsJson || "[]");
        } catch (e) {
            return [];
        }
    }
    readonly property int labelWidth: Kirigami.Units.gridUnit * 8

    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None
    pageStack.initialPage: Kirigami.Page {
        id: page
        title: details.name
        padding: Kirigami.Units.largeSpacing

        // ---- loading screen: one spinner until every stage has landed ----
        ColumnLayout {
            anchors.centerIn: parent
            visible: details.loading
            spacing: Kirigami.Units.largeSpacing

            Item {
                Layout.alignment: Qt.AlignHCenter
                implicitWidth: Kirigami.Units.iconSizes.enormous
                implicitHeight: Kirigami.Units.iconSizes.enormous

                QQC2.BusyIndicator {
                    anchors.fill: parent
                    running: details.loading
                }
                Kirigami.Icon {
                    anchors.centerIn: parent
                    width: Kirigami.Units.iconSizes.large
                    height: width
                    source: "com.ca-peterson.rclone-shell-ext"
                    fallback: "folder-cloud"
                }
            }
            QQC2.Label {
                Layout.alignment: Qt.AlignHCenter
                text: "Checking sync status…"
            }
        }

        // ---- the finished layout ----
        ColumnLayout {
            anchors.fill: parent
            visible: !details.loading
            spacing: Kirigami.Units.smallSpacing

            // 1. Identity
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: Kirigami.Units.iconSizes.large + Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.largeSpacing

                Kirigami.Icon {
                    source: details.iconName
                    fallback: details.isDir ? "folder" : "text-x-generic"
                    implicitWidth: Kirigami.Units.iconSizes.medium
                    implicitHeight: Kirigami.Units.iconSizes.medium
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Kirigami.Heading {
                        Layout.fillWidth: true
                        level: 3
                        text: details.name
                        elide: Text.ElideMiddle
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: details.path
                        elide: Text.ElideLeft
                        font: Kirigami.Theme.smallFont
                        opacity: 0.7
                    }
                }
            }

            Kirigami.Separator { Layout.fillWidth: true }

            // 2. Sync status: a title, a status line and a fixed set of detail rows
            Kirigami.Heading {
                level: 4
                text: "Sync status"
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: details.syncIcon
                    implicitWidth: Kirigami.Units.iconSizes.small
                    implicitHeight: Kirigami.Units.iconSizes.small
                }
                QQC2.Label {
                    text: details.syncLabel
                    elide: Text.ElideRight
                    Layout.maximumWidth: parent.width * 0.65
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignRight
                    text: details.syncDetail
                    elide: Text.ElideRight
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                }
            }
            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: Kirigami.Units.largeSpacing
                rowSpacing: Kirigami.Units.smallSpacing

                Repeater {
                    model: details.isDir
                        ? [["Uploading", details.uploading],
                           ["Downloading", details.downloading],
                           [details.cachedLabel, details.cached]]
                        : [["Drive modified", details.driveModified],
                           ["Cached locally", details.cacheState],
                           ["Size", details.size],
                           ["Version", details.version]]
                    delegate: Item {
                        id: cell
                        required property var modelData
                        Layout.columnSpan: 2
                        Layout.fillWidth: true
                        implicitHeight: row.implicitHeight
                        RowLayout {
                            id: row
                            anchors.left: parent.left
                            anchors.right: parent.right
                            spacing: Kirigami.Units.largeSpacing
                            QQC2.Label {
                                Layout.preferredWidth: root.labelWidth
                                text: cell.modelData[0]
                                font: Kirigami.Theme.smallFont
                                opacity: 0.7
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: cell.modelData[1]
                                elide: Text.ElideRight
                            }
                        }
                    }
                }
            }

            Kirigami.Separator { Layout.fillWidth: true }

            // 3. Sharing: a table of name / role / type that scrolls in the space left over
            RowLayout {
                Layout.fillWidth: true
                Kirigami.Heading {
                    Layout.fillWidth: true
                    level: 4
                    text: "Sharing"
                }
                QQC2.Label {
                    text: details.sharingSummary
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                }
            }
            RowLayout {
                Layout.fillWidth: true
                visible: details.sharingNote === ""
                spacing: Kirigami.Units.smallSpacing
                QQC2.Label {
                    Layout.fillWidth: true
                    text: "Name"
                    font.bold: true
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                }
                QQC2.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    text: "Role"
                    font.bold: true
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                }
                QQC2.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 8.5
                    text: "Type"
                    font.bold: true
                    font.pointSize: Kirigami.Theme.smallFont.pointSize
                }
                Item { implicitWidth: Kirigami.Units.gridUnit * 0.75 }
            }
            Kirigami.Separator {
                Layout.fillWidth: true
                visible: details.sharingNote === ""
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: details.sharingNote !== ""
                type: details.sharingNoteIcon === "dialog-warning-symbolic"
                    ? Kirigami.MessageType.Warning : Kirigami.MessageType.Information
                text: details.sharingNote
            }
            ListView {
                id: list
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: details.sharingNote === ""
                clip: true
                model: root.permissions
                QQC2.ScrollBar.vertical: QQC2.ScrollBar {}
                delegate: QQC2.ItemDelegate {
                    id: permRow
                    required property var modelData
                    width: ListView.view.width - Kirigami.Units.gridUnit * 0.75
                    hoverEnabled: true
                    QQC2.ToolTip.visible: hovered
                    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                    QQC2.ToolTip.text: permRow.modelData.full
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.smallSpacing
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: permRow.modelData.name
                                elide: Text.ElideRight
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                visible: permRow.modelData.address !== ""
                                text: permRow.modelData.address
                                elide: Text.ElideMiddle
                                font: Kirigami.Theme.smallFont
                                opacity: 0.7
                            }
                        }
                        QQC2.Label {
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                            text: permRow.modelData.role
                        }
                        RowLayout {
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 8.5
                            spacing: Kirigami.Units.smallSpacing
                            Kirigami.Icon {
                                source: permRow.modelData.icon
                                implicitWidth: Kirigami.Units.iconSizes.small
                                implicitHeight: Kirigami.Units.iconSizes.small
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: permRow.modelData.kind
                                elide: Text.ElideRight
                                font: Kirigami.Theme.smallFont
                            }
                        }
                    }
                }
            }
            // Pushes the offline row to the bottom when the table is replaced by a note.
            Item {
                Layout.fillHeight: true
                visible: details.sharingNote !== ""
            }

            // Offline: only when offline folders are on
            Kirigami.Separator {
                Layout.fillWidth: true
                visible: details.offlineEnabled
            }
            RowLayout {
                Layout.fillWidth: true
                visible: details.offlineEnabled
                spacing: Kirigami.Units.largeSpacing
                Kirigami.Icon {
                    source: "folder-download-symbolic"
                    implicitWidth: Kirigami.Units.iconSizes.small
                    implicitHeight: Kirigami.Units.iconSizes.small
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    QQC2.Label { text: details.offlineTitle }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: details.offlineCaption
                        elide: Text.ElideRight
                        font: Kirigami.Theme.smallFont
                        opacity: 0.7
                        color: details.offlineError !== ""
                            ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                    }
                }
                QQC2.Label {
                    visible: details.pinnedPhaseLabel !== ""
                    text: details.pinnedPhaseLabel
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                }
            }
        }

        // 4. Footer actions: present once loaded
        footer: QQC2.ToolBar {
            visible: !details.loading
            position: QQC2.ToolBar.Footer
            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing

                QQC2.Button {
                    id: openButton
                    text: "Open"
                    icon.name: "document-open"
                    onClicked: details.openIsMenu ? openMenu.popup(openButton, 0, -openMenu.height) : details.openFolder()

                    QQC2.Menu {
                        id: openMenu
                        QQC2.MenuItem {
                            text: "Folder"
                            icon.name: "folder-open"
                            onTriggered: details.openFolder()
                        }
                        QQC2.MenuItem {
                            text: "File"
                            icon.name: "document-open"
                            onTriggered: details.openFile()
                        }
                    }
                }
                Item { Layout.fillWidth: true }
                QQC2.Button {
                    visible: details.offlineAction !== "none"
                    text: details.offlineAction === "keep" ? "Keep offline" : "Stop keeping offline"
                    icon.name: details.offlineAction === "keep" ? "folder-download" : "edit-delete"
                    onClicked: details.keepOffline(details.offlineAction === "keep")
                }
                QQC2.Button {
                    highlighted: true
                    enabled: details.manageEnabled
                    text: details.manageLabel
                    icon.name: "document-share"
                    onClicked: details.manageSharing()
                }
            }
        }
    }
}
