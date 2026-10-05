import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

// The panel itself: what the fly is doing, and the buttons the Waybar module
// has no room for. Everything goes through the gnat binary's control socket
// (`gnat status`, `gnat toggle`, ...); nothing here talks to the sim directly.
Item {
  id: root

  required property var tokens
  property bool active: false
  signal dismissed()

  // ------------------------------------------------------------- tokens ---

  readonly property color fg: tokens.foreground
  readonly property color line: tint(fg, 0.15)
  readonly property color outline: tint(fg, 0.4)
  readonly property color faint: tint(fg, 0.04)
  readonly property string font: tokens.fontFamily

  function tint(c, a) {
    return Qt.rgba(c.r, c.g, c.b, a)
  }

  // --------------------------------------------------------------- state ---

  property string engine: ""
  property bool engineChecked: false
  readonly property bool engineMissing: engineChecked && engine === ""

  // Mirrors control.rs's MAX_FLIES; the binary enforces it, this just greys
  // out the button.
  readonly property int maxFlies: 64
  readonly property int maxReplyBytes: 4096
  readonly property int maxErrorChars: 300

  // `polled` flips on the first status reply; until then the state shows as "…".
  property bool running: false
  property bool polled: false
  property var status: ({})
  property string error: ""
  property bool starting: false

  readonly property int flies: running ? status.flies : 0

  function oneLine(text) {
    const first = String(text).trim().split("\n")[0]
    return first.length > maxErrorChars ? first.slice(0, maxErrorChars) + "…" : first
  }

  // The status line is shape-checked before use, so an old or broken binary
  // shows as "not answering" rather than as half-drawn numbers.
  function isStatus(s) {
    return s !== null && typeof s === "object"
      && typeof s.state === "string" && s.state.length < 64
      && typeof s.paused === "boolean" && typeof s.sleeping === "boolean"
      && typeof s.pop_hz === "number" && typeof s.neurons === "number"
      && typeof s.ledges === "number" && typeof s.flies === "number"
      && typeof s.brain === "boolean"
  }

  function stateLabel() {
    if (engineMissing) return "not installed"
    if (!polled) return "…"
    if (!running) return starting ? "starting" : "not running"
    if (status.paused) return "paused"
    if (status.sleeping) return "asleep"
    return status.state
  }

  // ----------------------------------------------------------- lifecycle ---

  function start() {
    error = ""
    if (engine) refresh()
    else resolveProc.running = true
    Qt.callLater(() => keys.forceActiveFocus())
  }

  function refresh() {
    if (engine && !statusProc.running) statusProc.running = true
  }

  // One command at a time; the socket answers in microseconds, so a click
  // that arrives mid-command is simply dropped.
  function send(args) {
    if (!engine || !running || cmdProc.running) return
    error = ""
    cmdProc.command = [engine].concat(args)
    cmdProc.running = true
  }

  function launch() {
    if (!engine || running || starting) return
    error = ""
    starting = true
    Quickshell.execDetached([engine, "--run"])
    startTimeout.restart()
  }

  function setFlies(n) {
    if (!running) return
    send(["flies", String(Math.max(1, Math.min(maxFlies, n)))])
  }

  // ----------------------------------------------------------- processes ---

  // The binary is found the way a user's own shell would find it, never via a
  // folder the host hands over. Printing nothing means it is not installed.
  Process {
    id: resolveProc
    command: ["sh", "-c", "if [ -n \"$GNAT_BIN\" ] && [ -x \"$GNAT_BIN\" ]; then printf %s \"$GNAT_BIN\"; "
      + "elif command -v gnat >/dev/null 2>&1; then command -v gnat; "
      + "elif [ -x \"$HOME/.local/bin/gnat\" ]; then printf %s \"$HOME/.local/bin/gnat\"; fi"]
    stdout: StdioCollector {
      onStreamFinished: {
        const path = text.trim()
        root.engine = path.startsWith("/") && !path.includes("\n") ? path : ""
        root.engineChecked = true
        root.refresh()
      }
    }
  }

  // `gnat status` exits non-zero when nothing is listening on the socket.
  Process {
    id: statusProc
    command: [root.engine, "status"]
    stdout: StdioCollector { id: statusOut }
    onExited: function(code) {
      root.polled = true
      let s = null
      if (code === 0 && statusOut.text.length <= root.maxReplyBytes) {
        try { s = JSON.parse(statusOut.text) } catch (e) { s = null }
      }
      // status before running: the bindings that read it wake on running.
      const ok = root.isStatus(s)
      if (ok) root.status = s
      root.running = ok
      if (ok) {
        root.starting = false
        startTimeout.stop()
      } else if (code === 0) {
        root.error = "gnat answered status with something unexpected — is it up to date?"
      }
    }
  }

  Process {
    id: cmdProc
    stdout: StdioCollector { id: cmdOut }
    stderr: StdioCollector { id: cmdErr }
    onExited: function(code) {
      const reply = cmdOut.text.trim()
      if (code !== 0) root.error = root.oneLine(cmdErr.text) || "gnat failed"
      else if (reply.startsWith("error")) root.error = root.oneLine(reply)
      root.refresh()
    }
  }

  Timer {
    interval: 1000
    repeat: true
    running: root.active && root.engine !== ""
    onTriggered: root.refresh()
  }

  // A fly that has not answered in a few seconds did not start; say so
  // instead of showing "starting" forever.
  Timer {
    id: startTimeout
    interval: 6000
    onTriggered: {
      root.starting = false
      if (!root.running) root.error = "gnat did not start — run `gnat` in a terminal to see why"
    }
  }

  // ------------------------------------------------------------ keyboard ---

  Item {
    id: keys
    focus: true
    Keys.onPressed: function(event) {
      const key = event.key
      const text = event.text
      if (key === Qt.Key_Escape) {
        root.dismissed()
      } else if (!root.running && (key === Qt.Key_Return || key === Qt.Key_Enter)) {
        root.launch()
      } else if (key === Qt.Key_Space) {
        root.send(["toggle"])
      } else if (text === "s") {
        root.send(["scare"])
      } else if (text === "b") {
        root.send(["brain"])
      } else if (text === "-" || text === "[") {
        root.setFlies(root.flies - 1)
      } else if (text === "+" || text === "=" || text === "]") {
        root.setFlies(root.flies + 1)
      } else if (text === "q") {
        root.send(["quit"])
      } else {
        return
      }
      event.accepted = true
    }
  }

  // -------------------------------------------------------------- layout ---

  Rectangle {
    anchors.fill: parent
    color: root.tokens.scrim
  }

  // A click outside the card closes it.
  MouseArea {
    anchors.fill: parent
    onClicked: root.dismissed()
  }

  Rectangle {
    id: card
    anchors.centerIn: parent
    width: Math.min(parent.width - 48, 520)
    height: body.implicitHeight + 2 * card.border.width
    color: root.tokens.background
    border.color: root.tokens.border
    border.width: root.tokens.borderWidth
    radius: root.tokens.radius
    clip: true

    MouseArea { anchors.fill: parent }

    ColumnLayout {
      id: body
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: card.border.width
      spacing: 0

      // Header: name and what the fly is doing right now.
      RowLayout {
        Layout.fillWidth: true
        Layout.preferredHeight: 48
        Layout.leftMargin: 20
        Layout.rightMargin: 20
        spacing: 16

        Text {
          textFormat: Text.PlainText
          text: "gnat"
          color: root.fg
          font.family: root.font
          font.pixelSize: 14
          font.bold: true
        }
        Text {
          textFormat: Text.PlainText
          text: "connectome fly"
          color: root.tokens.muted
          font.family: root.font
          font.pixelSize: 13
        }
        Item { Layout.fillWidth: true }
        Text {
          textFormat: Text.PlainText
          text: root.stateLabel()
          color: root.running && !root.status.paused ? root.tokens.accent : root.tokens.muted
          font.family: root.font
          font.pixelSize: 13
          font.bold: true
        }
      }

      Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: root.line }

      // Not installed / not running: one line saying so, and how to fix it.
      Text {
        visible: !root.running
        Layout.fillWidth: true
        Layout.margins: 20
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        text: root.engineMissing
          ? "The gnat binary is not on PATH or in ~/.local/bin. Build it (cargo build --release) and link target/release/gnat into ~/.local/bin."
          : "No fly on the screen. Start one here, or enable the systemd unit in packaging/ to have one every login."
        color: root.tokens.muted
        font.family: root.font
        font.pixelSize: 13
      }

      // Readings from `gnat status`.
      ColumnLayout {
        visible: root.running
        Layout.fillWidth: true
        Layout.margins: 20
        spacing: 8

        Repeater {
          model: root.running ? [
            ["flies", String(root.status.flies)],
            ["population rate", root.status.pop_hz.toFixed(2) + " Hz / neuron"],
            ["neurons", String(root.status.neurons)],
            ["standing on", root.status.ledges + " ledges in view"],
            ["brain view", root.status.brain ? "open" : "closed"]
          ] : []
          delegate: Item {
            required property var modelData
            Layout.fillWidth: true
            implicitHeight: valueText.implicitHeight
            Text {
              textFormat: Text.PlainText
              text: modelData[0]
              color: root.tokens.muted
              font.family: root.font
              font.pixelSize: 13
            }
            Text {
              id: valueText
              anchors.right: parent.right
              textFormat: Text.PlainText
              text: modelData[1]
              color: root.fg
              font.family: root.font
              font.pixelSize: 13
            }
          }
        }
      }

      Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: root.line }

      // Actions.
      Flow {
        Layout.fillWidth: true
        Layout.margins: 20
        spacing: 8

        FlatButton {
          ui: root
          visible: !root.running && !root.engineMissing
          primary: true
          label: root.starting ? "starting…" : "start"
          onActivated: root.launch()
        }
        FlatButton {
          ui: root
          visible: root.running
          primary: true
          label: root.status.paused ? "resume" : "pause"
          onActivated: root.send(["toggle"])
        }
        FlatButton {
          ui: root
          visible: root.running
          label: "scare"
          onActivated: root.send(["scare"])
        }
        FlatButton {
          ui: root
          visible: root.running
          label: root.status.brain ? "brain open" : "brain"
          checked: root.running && root.status.brain
          onActivated: root.send(["brain"])
        }
        FlatButton {
          ui: root
          visible: root.running
          label: "−"
          implicitWidth: 36
          onActivated: root.setFlies(root.flies - 1)
          opacity: root.flies > 1 ? 1 : 0.4
        }
        FlatButton {
          ui: root
          visible: root.running
          label: "+"
          implicitWidth: 36
          onActivated: root.setFlies(root.flies + 1)
          opacity: root.flies < root.maxFlies ? 1 : 0.4
        }
        FlatButton {
          ui: root
          visible: root.running
          label: "quit fly"
          onActivated: root.send(["quit"])
        }
      }

      Text {
        visible: root.error !== ""
        Layout.fillWidth: true
        Layout.leftMargin: 20
        Layout.rightMargin: 20
        Layout.bottomMargin: 16
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        text: root.error
        color: root.tokens.urgent
        font.family: root.font
        font.pixelSize: 12
      }

      Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: root.line }

      // Keys.
      Flow {
        Layout.fillWidth: true
        Layout.margins: 14
        Layout.leftMargin: 20
        spacing: 14

        Repeater {
          model: root.running
            ? [["space", "pause"], ["s", "scare"], ["b", "brain"], ["- +", "flies"], ["q", "quit"], ["esc", "close"]]
            : [["enter", "start"], ["esc", "close"]]
          delegate: Row {
            required property var modelData
            spacing: 6
            Rectangle {
              width: keyText.implicitWidth + 12
              height: keyText.implicitHeight + 4
              color: "transparent"
              border.width: 1
              border.color: root.tint(root.fg, 0.25)
              radius: root.tokens.radius
              Text {
                id: keyText
                textFormat: Text.PlainText
                anchors.centerIn: parent
                text: modelData[0]
                color: root.fg
                font.family: root.font
                font.pixelSize: 12
              }
            }
            Text {
              textFormat: Text.PlainText
              anchors.verticalCenter: parent.verticalCenter
              text: modelData[1]
              color: root.tokens.muted
              font.family: root.font
              font.pixelSize: 12
            }
          }
        }
      }
    }
  }
}
