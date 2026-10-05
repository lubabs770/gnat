import QtQuick

// Outlined, square, monospace — the Omarchy control look. `ui` is the Panel,
// which owns the colour tokens.
Rectangle {
  id: button

  required property var ui
  property string label: ""
  property bool primary: false
  property bool checked: false
  property bool checkable: false
  property int fontSize: 13
  signal activated()

  implicitHeight: 36
  implicitWidth: text.implicitWidth + 32
  radius: ui.tokens.radius
  border.width: 1
  border.color: primary || checked ? ui.tokens.accent : ui.outline
  color: primary ? ui.tokens.accent
    : checked || mouse.containsMouse ? ui.tint(ui.fg, 0.08)
    : ui.faint

  Accessible.role: Accessible.Button
  Accessible.name: label
  Accessible.checkable: checkable
  Accessible.checked: checked

  Text {
    id: text
    textFormat: Text.PlainText
    anchors.centerIn: parent
    text: button.label
    color: button.primary ? button.ui.tokens.background
      : button.checked ? button.ui.tokens.accent
      : button.ui.fg
    font.family: button.ui.font
    font.pixelSize: button.fontSize
    font.bold: button.primary
  }

  MouseArea {
    id: mouse
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: button.activated()
  }
}
