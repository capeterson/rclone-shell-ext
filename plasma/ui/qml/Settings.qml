// SPDX-License-Identifier: GPL-3.0-only
// The settings window (`rclone-shell-ext-plasma --settings`): offline folders setup and the pinned list,
// folder backups and restoring them, plus the Dolphin integration.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigamiaddons.formcard as FormCard
import com.ca_peterson.RcloneShellExt

Kirigami.ApplicationWindow {
    id: root

    title: Strings.t("settings-window-title")
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
    readonly property var backups: parse(settings.backupsJson)
    readonly property var restores: parse(settings.restoreJson)

    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None
    pageStack.initialPage: FormCard.FormCardPage {
        title: Strings.t("settings-window-title")

        // Offline folders
        FormCard.FormHeader { title: Strings.t("offline-heading") }
        FormCard.FormCard {
            FormCard.FormSwitchDelegate {
                text: Strings.t("settings-keep-offline-switch")
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
                            text: step.modelData.done ? Strings.t("settings-step-done") : Strings.t("settings-step-todo")
                            font: Kirigami.Theme.smallFont
                        }
                    }
                }
            }
            FormCard.FormButtonDelegate {
                visible: settings.showSetup
                enabled: !settings.busy
                text: settings.busy ? Strings.t("settings-setting-up") : Strings.t("settings-setup-incomplete")
                description: settings.setupHint
                icon.name: "run-build"
                onClicked: settings.requestSetup()
            }

            // The folders kept offline
            FormCard.FormPlaceholderMessageDelegate {
                visible: settings.showPins && settings.noPins
                text: Strings.t("settings-no-pins")
                icon.name: "folder-download"
                // Shown as the card's body: how to add one.
            }
            FormCard.FormTextDelegate {
                visible: settings.showPins && settings.noPins
                description: Strings.t("plasma-settings-no-pins-hint")
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
                            text: Strings.t("common-repair")
                            icon.name: "tools-wizard"
                            onClicked: settings.repair(pinRow.modelData.rel)
                        }
                        QQC2.Button {
                            visible: !pinRow.modelData.removing
                            text: Strings.t("common-open")
                            icon.name: "document-open-folder"
                            onClicked: settings.openPin(pinRow.modelData.rel)
                        }
                        QQC2.Button {
                            visible: !pinRow.modelData.removing
                            text: Strings.t("common-remove")
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
            title: Strings.t("settings-heading-activity")
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

        // Folder backups
        FormCard.FormHeader { title: Strings.t("settings-heading-backups") }
        FormCard.FormCard {
            FormCard.FormTextDelegate {
                description: settings.backupDescription
            }
            FormCard.FormTextDelegate {
                visible: root.backups.length === 0
                description: Strings.t("settings-backup-none")
            }
            Repeater {
                model: root.backups
                delegate: FormCard.AbstractFormDelegate {
                    id: backupRow
                    required property var modelData
                    background: null
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing
                        Kirigami.Icon {
                            source: backupRow.modelData.error ? "dialog-error-symbolic" : "folder-cloud"
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: backupRow.modelData.source
                                elide: Text.ElideMiddle
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: backupRow.modelData.destination
                                elide: Text.ElideMiddle
                                font: Kirigami.Theme.smallFont
                                opacity: 0.7
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                text: backupRow.modelData.status
                                wrapMode: Text.Wrap
                                font: Kirigami.Theme.smallFont
                                color: backupRow.modelData.error ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                                opacity: backupRow.modelData.error ? 1 : 0.7
                            }
                        }
                        QQC2.Button {
                            text: Strings.t("backup-menu-remove")
                            icon.name: "list-remove"
                            enabled: !settings.backupBusy
                            onClicked: settings.stopBackup(backupRow.modelData.source)
                        }
                    }
                }
            }
            FormCard.FormButtonDelegate {
                text: Strings.t("settings-backup-add")
                icon.name: "list-add"
                enabled: !settings.backupBusy
                onClicked: backupDialog.open()
            }
        }

        // Restoring a backup (this machine's or another's) to any local folder
        FormCard.FormHeader { title: Strings.t("settings-restore-heading") }
        FormCard.FormCard {
            FormCard.FormButtonDelegate {
                text: settings.restoreLoading ? Strings.t("settings-restore-loading") : Strings.t("settings-restore-load")
                icon.name: "edit-find"
                enabled: !settings.restoreLoading
                onClicked: settings.findBackups()
            }
            FormCard.FormTextDelegate {
                visible: settings.restoreLoaded && !settings.restoreLoading && root.restores.length === 0
                text: Strings.t("settings-restore-none")
            }
            Repeater {
                model: root.restores
                delegate: FormCard.AbstractFormDelegate {
                    id: restoreRow
                    required property var modelData
                    background: null
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing
                        Kirigami.Icon {
                            source: "folder-cloud"
                            implicitWidth: Kirigami.Units.iconSizes.small
                            implicitHeight: Kirigami.Units.iconSizes.small
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: restoreRow.modelData.label
                            elide: Text.ElideMiddle
                        }
                        QQC2.Button {
                            text: Strings.t("settings-restore-button")
                            icon.name: "document-import"
                            onClicked: {
                                restoreDialog.destination = restoreRow.modelData.destination;
                                restoreDialog.open();
                            }
                        }
                    }
                }
            }
            FormCard.FormCheckDelegate {
                id: restoreLink
                visible: root.restores.length > 0
                text: Strings.t("settings-restore-link")
            }
        }

        // Dolphin integration
        FormCard.FormHeader { title: Strings.t("plasma-settings-dolphin-heading") }
        FormCard.FormCard {
            FormCard.FormSwitchDelegate {
                text: Strings.t("plasma-settings-dolphin-menu-switch")
                checked: settings.contextMenu
                onToggled: {
                    settings.saveContextMenu(checked);
                    checked = Qt.binding(() => settings.contextMenu);
                }
            }
            FormCard.FormTextDelegate {
                text: Strings.t("plasma-settings-dolphin-status-title")
                description: Strings.t("plasma-settings-dolphin-status-detail")
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

    FolderDialog {
        id: backupDialog
        title: Strings.t("settings-backup-choose")
        onAccepted: settings.addBackup(backupDialog.selectedFolder)
    }

    FolderDialog {
        id: restoreDialog
        // The backup the dialog was opened for (its rclone path).
        property string destination: ""
        title: Strings.t("settings-restore-choose")
        onAccepted: settings.restoreBackup(restoreDialog.destination, restoreDialog.selectedFolder, restoreLink.checked)
    }

    // Informed consent before setup changes anything: what will change, then Continue (on to
    // the password prompt) or Cancel (nothing is changed and the switch stays off).
    Kirigami.Dialog {
        id: consentDialog
        visible: settings.consentVisible
        title: Strings.t("settings-consent-title")
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
                text: Strings.t("common-cancel")
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
                text: Strings.t("settings-consent-intro")
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
