import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons

// omafly: gnat inside Omarchy, an overlay plugin in the running omarchy-shell,
// coloured by the shell's live theme tokens (the same [menu] surface the
// clipboard and menu use). It is only a remote for the fly — the fly itself is
// the gnat binary, reached over its control socket.
Item {
  id: root

  readonly property string pluginId: "io.github.lubabs770.omafly"

  // Handed over by the shell's plugin loader.
  property var shell: null
  property var manifest: null
  property string omarchyPath: ""

  property bool opened: false

  Tokens {
    id: themeTokens
    background: Color.menu.background
    foreground: Color.menu.text
    accent: Color.accent
    muted: Color.muted
    urgent: Color.urgent
    scrim: Color.menu.scrim
    border: Color.menu.border
    selectedBackground: Color.menu.selectedBackground
    selectedText: Color.menu.selectedText
    borderWidth: Math.max(1, Style.space(2))
    radius: Style.cornerRadius
    fontFamily: Style.font.menuFamily
  }

  function open(payloadJson) {
    if (root.opened) return
    root.opened = true
    panel.start()
  }

  // Nothing to undo: every button acts immediately, so closing is just closing.
  function close() {
    root.opened = false
  }

  function toggle() {
    if (root.opened) root.dismiss()
    else root.open("{}")
  }

  // Closing from inside: tell the shell, so it unloads us and its own record
  // of what is open stays true.
  function dismiss() {
    root.opened = false
    if (root.shell && typeof root.shell.hide === "function") root.shell.hide(root.pluginId)
  }

  PanelWindow {
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.namespace: "omafly"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive

    Panel {
      id: panel
      anchors.fill: parent
      tokens: themeTokens
      active: root.opened
      onDismissed: root.dismiss()
    }
  }
}
