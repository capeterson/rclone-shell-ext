import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Item {
    id: compact

    property string iconName: "rclone-plasma-loading"
    signal toggle()

    Layout.minimumWidth: Kirigami.Units.iconSizes.small
    Layout.minimumHeight: Kirigami.Units.iconSizes.small

    Kirigami.Icon {
        anchors.fill: parent
        source: compact.iconName
        active: mouse.containsMouse
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton
        onClicked: compact.toggle()
    }
}
