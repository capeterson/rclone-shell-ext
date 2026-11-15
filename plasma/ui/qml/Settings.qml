// SPDX-License-Identifier: GPL-3.0-only
// The settings window (`rclone-plasma --settings`): offline folders setup and the pinned list,
// plus the Dolphin integration.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.formcard as FormCard
import com.ca_peterson.RclonePlasma

Kirigami.ApplicationWindow {
    id: root

    title: "rclone settings"
    width: Kirigami.Units.gridUnit * 36
    height: Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 24
    minimumHeight: Kirigami.Units.gridUnit * 24

    SettingsModel {
        id: settings
        Component.onCompleted: settings.start()
    }

    function parse(text) {
        try {
            return JSON.parse(text || "[]");
        } catch (e) {
            return [];
        }
    }
    readonly property var consentItems: parse(settings.consentJson)
    readonly property var setupSteps: parse(settings.setupStepsJson)
    readonly property var pins: parse(settings.pinsJson)
    readonly property var notices: parse(settings.noticesJson)

    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None
    pageStack.initialPage: FormCard.FormCardPage {
        title: "rclone settings"

        // Offline folders
        FormCard.FormHeader { title: "Offline folders" }
        FormCard.FormCard {
            FormCard.FormSwitchDelegate {
                text: "Keep folders available offline"
                description: settings.description
                // Bound to the model, and flipping it is a request, not a change: the model
                // decides (consent first) and the binding is restored from its state.
                checked: settings.toggleChecked
                enabled: settings.toggleEnabled
                onToggled: {
                    settings.toggleOffline(checked);
                    checked = Qt.binding(() => settings.toggleChecked);
                }
            }

            // The setup checklist, shown while setup is running or incomplete
            Repeater {
                model: settings.showSetup ? root.setupSteps : []
                delegate: FormCard.FormTextDelegate {
                    id: step
                    required property var modelData
                    text: step.modelData.title
                    description: step.modelData.detail
                    trailing: RowLayout {
                        spacing: Kirigami.Units.smallSpacing
                        Kirigami.Icon {
                            source: step.modelData.done ? "emblem-ok-symbolic" : "dialog-information-symbolic"
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                        }
                        QQC2.Label {
                            text: step.modelData.done ? "Done" : "To do"
                            font: Kirigami.Theme.smallFont
                        }
                    }
                }
            }
            FormCard.FormButtonDelegate {
                visible: settings.showSetup
                enabled: !settings.busy
                text: settings.busy ? "Setting up…" : "Setup isn't complete"
                description: settings.setupHint
                icon.name: "run-build"
                onClicked: settings.requestSetup()
            }

            // The folders kept offline
            FormCard.FormPlaceholderMessageDelegate {
                visible: settings.showPins && settings.noPins
                text: "No folders are kept offline yet"
                icon.name: "folder-download"
                // Shown as the card's body: how to add one.
            }
            FormCard.FormTextDelegate {
                visible: settings.showPins && settings.noPins
                description: "In Dolphin, right-click a folder in the Drive folder, choose “rclone”, then “Keep offline”."
            }
            Repeater {
                model: settings.showPins ? root.pins : []
                delegate: FormCard.AbstractFormDelegate {
                    id: pinRow
                    required property var modelData
                    background: null
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing
                        Kirigami.Icon {
                            source: pinRow.modelData.icon
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: pinRow.modelData.rel
                                elide: Text.ElideMiddle
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: pinRow.modelData.label
                                wrapMode: Text.Wrap
                                font: Kirigami.Theme.smallFont
                                opacity: 0.7
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                visible: pinRow.modelData.error !== ""
                                text: pinRow.modelData.error
                                wrapMode: Text.Wrap
                                font: Kirigami.Theme.smallFont
                                color: Kirigami.Theme.negativeTextColor
                            }
                        }
                        QQC2.Button {
                            visible: pinRow.modelData.canRepair
                            text: "Repair"
                            icon.name: "tools-wizard"
                            onClicked: settings.repair(pinRow.modelData.rel)
                        }
                        QQC2.Button {
                            visible: !pinRow.modelData.removing
                            text: "Open"
                            icon.name: "document-open-folder"
                            onClicked: settings.openPin(pinRow.modelData.rel)
                        }
                        QQC2.Button {
                            visible: !pinRow.modelData.removing
                            text: "Remove"
                            icon.name: "edit-delete"
                            onClicked: settings.remove(pinRow.modelData.rel)
                        }
                    }
                }
            }
        }

        // Recent offline activity
        FormCard.FormHeader {
            visible: settings.offlineEnabled && root.notices.length > 0
            title: "Recent offline activity"
        }
        FormCard.FormCard {
            visible: settings.offlineEnabled && root.notices.length > 0
            Repeater {
                model: root.notices
                delegate: FormCard.FormTextDelegate {
                    id: notice
                    required property var modelData
                    text: notice.modelData.text
                    description: notice.modelData.when
                }
            }
        }

        // Dolphin integration
        FormCard.FormHeader { title: "Dolphin integration" }
        FormCard.FormCard {
            FormCard.FormSwitchDelegate {
                text: "Show “rclone” in Dolphin's right-click menu"
                checked: settings.contextMenu
                onToggled: {
                    settings.saveContextMenu(checked);
                    checked = Qt.binding(() => settings.contextMenu);
                }
            }
            FormCard.FormTextDelegate {
                text: "Sync status in Dolphin"
                description: "Dolphin's plugins show sync emblems on files in the Drive folder and add an “rclone” action for details, offline folders and sharing. They talk to the rclone-plasma background service over D-Bus."
            }
        }

        // Outcome of the last change
        FormCard.FormCard {
            visible: settings.error !== "" || settings.note !== ""
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: settings.error !== ""
                type: Kirigami.MessageType.Error
                text: settings.error
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: settings.note !== ""
                type: Kirigami.MessageType.Information
                text: settings.note
            }
        }
    }

    // Informed consent before setup changes anything: what will change, then Continue (on to
    // the password prompt) or Cancel (nothing is changed and the switch stays off).
    Kirigami.Dialog {
        id: consentDialog
        visible: settings.consentVisible
        title: "Turn on offline folders?"
        padding: Kirigami.Units.largeSpacing
        preferredWidth: Kirigami.Units.gridUnit * 28
        closePolicy: QQC2.Popup.CloseOnEscape
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: settings.consentGoLabel
                icon.name: "dialog-ok"
                onTriggered: settings.answerConsent(true)
            },
            Kirigami.Action {
                text: "Cancel"
                icon.name: "dialog-cancel"
                onTriggered: settings.answerConsent(false)
            }
        ]
        onRejected: if (settings.consentVisible) settings.answerConsent(false)

        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: "Keeping folders offline needs these changes. Nothing has been changed yet."
            }
            Repeater {
                model: root.consentItems
                delegate: ColumnLayout {
                    id: item
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 0
                    Kirigami.Heading {
                        level: 4
                        Layout.fillWidth: true
                        text: item.modelData.title
                        wrapMode: Text.Wrap
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        text: item.modelData.detail
                        wrapMode: Text.Wrap
                    }
                }
            }
        }
    }
}
