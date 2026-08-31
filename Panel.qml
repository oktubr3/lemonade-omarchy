import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Panel de búsqueda de Lemonade, keyboard-first como el launcher:
//   escribir  → filtra
//   ↑/↓       → navega
//   Enter     → copia la contraseña (auto-clear 30s, lo hace el CLI)
//   Ctrl+↵    → copia el usuario
//   Alt+↵     → copia el código TOTP
//   Shift+↵   → cierra y tipea la contraseña en la ventana enfocada
//   Esc       → cierra
//
// El panel nunca ve una contraseña: el CLI habla con el backend y va
// directo a wl-copy/wtype por stdin. Acá solo viaja metadata.
Panel {
  id: root
  moduleName: "io.github.oktubr3.lemonade"
  ipcTarget: moduleName
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property color contentForeground: bar ? bar.foreground : Color.foreground
  readonly property color mutedForeground: Qt.rgba(contentForeground.r, contentForeground.g, contentForeground.b, 0.55)
  readonly property color rowFill: Qt.rgba(contentForeground.r, contentForeground.g, contentForeground.b, 0.045)
  readonly property color rowBorder: Qt.rgba(contentForeground.r, contentForeground.g, contentForeground.b, 0.18)

  property var entries: []
  property var filtered: []
  property int selectedIndex: 0
  property string statusText: ""
  property bool needsLogin: false
  property bool busy: false

  onOpenedChanged: {
    if (opened) {
      pendingDelete = null
      statusText = ""
      needsLogin = false
      searchField.text = ""
      selectedIndex = 0
      listProc.refresh = false
      listProc.running = true          // cache: instantáneo
      refreshTimer.start()             // red: actualiza atrás
    }
  }

  function applyFilter() {
    var q = searchField.text.toLowerCase().trim()
    if (q === "") {
      filtered = entries
    } else {
      var terms = q.split(/\s+/)
      filtered = entries.filter(function(e) {
        var hay = ((e.title || "") + " " + (e.username || "") + " " + (e.url || "")).toLowerCase()
        return terms.every(function(t) { return hay.indexOf(t) !== -1 })
      })
    }
    if (selectedIndex >= filtered.length) selectedIndex = Math.max(0, filtered.length - 1)
  }

  function current() {
    return filtered.length > 0 && selectedIndex < filtered.length ? filtered[selectedIndex] : null
  }

  function move(delta) {
    if (filtered.length === 0) return
    selectedIndex = Math.min(Math.max(selectedIndex + delta, 0), filtered.length - 1)
    entryList.positionViewAtIndex(selectedIndex, ListView.Contain)
  }

  function runAction(args, closeAfter) {
    if (busy) return
    busy = true
    actionProc.command = ["lemonade"].concat(args)
    actionProc.running = true
    if (closeAfter) root.close()
  }

  function copyPassword() {
    var e = current(); if (!e) return
    statusText = "Copiando contraseña de " + e.title + "…"
    runAction(["copy", e.id], false)
  }

  function copyUsername() {
    var e = current(); if (!e) return
    statusText = "Copiando usuario de " + e.title + "…"
    runAction(["copy", e.id, "--field", "username"], false)
  }

  function copyUrl() {
    var e = current(); if (!e) return
    if (!e.url || e.url === "") { statusText = e.title + " no tiene URL"; return }
    statusText = "Copiando URL de " + e.title + "…"
    runAction(["copy", e.id, "--field", "url"], false)
  }

  function copyTotp() {
    var e = current(); if (!e) return
    if (!e.has_totp) { statusText = e.title + " no tiene TOTP"; return }
    statusText = "Pidiendo TOTP de " + e.title + "…"
    runAction(["totp", e.id], false)
  }

  property var pendingDelete: null

  function armDelete() {
    var e = current(); if (!e) return
    pendingDelete = e
    statusText = "¿Mandar \"" + e.title + "\" a la papelera?  Enter confirma · Esc cancela"
  }

  function cancelDelete() {
    if (!pendingDelete) return
    pendingDelete = null
    statusText = ""
  }

  function confirmDelete() {
    if (!pendingDelete) return
    var e = pendingDelete
    pendingDelete = null
    statusText = "Borrando \"" + e.title + "\"…"
    runAction(["rm", e.id, "--yes"], false)
  }

  function editEntry() {
    var e = current(); if (!e) return
    root.close()
    Util.execDetached("omarchy-launch-floating-terminal-with-presentation " +
      "\"lemonade edit " + e.id + "; echo; echo 'Enter para cerrar'; read -r\"")
  }

  function autotype() {
    var e = current(); if (!e) return
    // Cerrar primero: el foco vuelve a la ventana anterior y el CLI
    // espera 650ms antes de tipear.
    runAction(["type", e.id, "--delay", "650"], true)
  }

  // --- procesos ---

  Process {
    id: listProc
    property bool refresh: false
    command: refresh ? ["lemonade", "list", "--json", "--refresh"]
                     : ["lemonade", "list", "--json"]
    stdout: StdioCollector {
      id: listOut
      waitForEnd: true
    }
    stderr: StdioCollector {
      id: listErr
      waitForEnd: true
    }
    onExited: function(exitCode) {
      if (exitCode === 0) {
        try {
          root.entries = JSON.parse(listOut.text)
          root.needsLogin = false
          root.applyFilter()
        } catch (e) {
          root.statusText = "Respuesta ilegible del CLI"
        }
      } else {
        var err = String(listErr.text).trim()
        if (err.indexOf("sin sesión") !== -1 || err.indexOf("login") !== -1) {
          root.needsLogin = true
        } else if (root.entries.length === 0) {
          root.statusText = err !== "" ? err : "lemonade list falló"
        }
      }
    }
  }

  Timer {
    id: refreshTimer
    interval: 150
    repeat: false
    onTriggered: {
      if (!listProc.running) {
        listProc.refresh = true
        listProc.running = true
      }
    }
  }

  Process {
    id: actionProc
    stderr: StdioCollector {
      id: actionErr
      waitForEnd: true
    }
    onExited: function(exitCode) {
      root.busy = false
      var msg = String(actionErr.text).trim()
      if (exitCode === 0) {
        root.statusText = msg !== "" ? msg : "Listo."
        statusClear.restart()
        if (!listProc.running) { listProc.refresh = false; listProc.running = true }
      } else {
        root.statusText = msg !== "" ? msg : "Falló el comando"
      }
    }
  }

  Timer {
    id: statusClear
    interval: 4000
    repeat: false
    onTriggered: root.statusText = ""
  }

  // --- UI ---

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    focusTarget: searchField
    contentWidth: panel.fittedContentWidth(Style.space(420))
    contentHeight: panel.fittedContentHeight(contentColumn.implicitHeight, Style.space(520))

    Column {
      id: contentColumn
      width: parent.width
      spacing: Style.space(6)

      Item {
        width: parent.width
        height: Style.space(22)

        Row {
          spacing: Style.space(6)
          anchors.verticalCenter: parent.verticalCenter
          Text {
            text: "\uf094"
            color: root.contentForeground
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
            anchors.verticalCenter: parent.verticalCenter
          }
          Text {
            text: "Lemonade"
            color: root.contentForeground
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
            font.bold: true
            anchors.verticalCenter: parent.verticalCenter
          }
        }

        // "+": alta interactiva en terminal flotante (la contraseña se
        // ingresa oculta ahí; el panel jamás la ve).
        Rectangle {
          visible: !root.needsLogin
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(22)
          height: Style.space(22)
          radius: Style.space(5)
          color: addArea.containsMouse ? root.rowFill : "transparent"
          border.color: addArea.containsMouse ? root.rowBorder : "transparent"
          border.width: 1

          Text {
            anchors.centerIn: parent
            text: "+"
            color: root.contentForeground
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
          }
          MouseArea {
            id: addArea
            anchors.fill: parent
            hoverEnabled: true
            onClicked: {
              root.close()
              Util.execDetached("omarchy-launch-floating-terminal-with-presentation " +
                "\"lemonade add; echo; echo 'Enter para cerrar'; read -r\"")
            }
          }
        }
      }

      TextField {
        id: searchField
        width: parent.width
        placeholderText: root.needsLogin ? "Sin sesión" : "Buscar…"
        enabled: !root.needsLogin
        onTextChanged: { root.cancelDelete(); root.selectedIndex = 0; root.applyFilter() }

        Keys.onUpPressed: { root.cancelDelete(); root.move(-1) }
        Keys.onDownPressed: { root.cancelDelete(); root.move(1) }
        Keys.onEscapePressed: {
          if (root.pendingDelete) root.cancelDelete()
          else root.close()
        }
        Keys.onDeletePressed: function(event) {
          if (event.modifiers & Qt.ControlModifier) root.armDelete()
          else event.accepted = false   // Delete a secas sigue editando el texto
        }
        Keys.onPressed: function(event) {
          if (!(event.modifiers & Qt.ControlModifier)) return
          if (event.key === Qt.Key_E) { root.editEntry(); event.accepted = true }
          else if (event.key === Qt.Key_U) { root.copyUsername(); event.accepted = true }
          else if (event.key === Qt.Key_L) { root.copyUrl(); event.accepted = true }
        }
        function dispatchEnter(mods) {
          if (root.pendingDelete) { root.confirmDelete(); return }
          if (mods & Qt.ShiftModifier) root.autotype()
          else if (mods & Qt.ControlModifier) root.copyUsername()
          else if (mods & Qt.AltModifier) root.copyTotp()
          else root.copyPassword()
        }
        Keys.onReturnPressed: function(event) { dispatchEnter(event.modifiers) }
        Keys.onEnterPressed: function(event) { dispatchEnter(event.modifiers) }
      }

      // Sin sesión: instrucción en lugar de lista
      Column {
        width: parent.width
        visible: root.needsLogin
        spacing: Style.space(4)
        Text {
          width: parent.width
          wrapMode: Text.WordWrap
          text: "No hay sesión activa.\nCorré en una terminal:  lemonade login"
          color: root.contentForeground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }

      ListView {
        id: entryList
        width: parent.width
        height: Math.min(contentHeight, Style.space(360))
        visible: !root.needsLogin
        clip: true
        interactive: true
        boundsBehavior: Flickable.StopAtBounds
        model: root.filtered
        currentIndex: root.selectedIndex

        delegate: Rectangle {
          required property var modelData
          required property int index
          width: entryList.width
          height: Style.space(34)
          radius: Style.space(5)
          color: index === root.selectedIndex ? root.rowFill : "transparent"
          border.color: index === root.selectedIndex ? root.rowBorder : "transparent"
          border.width: 1

          MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onEntered: root.selectedIndex = index
            onClicked: function(mouse) {
              root.selectedIndex = index
              if (mouse.button === Qt.RightButton) root.editEntry()
              else root.copyPassword()
            }
          }

          Row {
            anchors.fill: parent
            anchors.leftMargin: Style.space(8)
            anchors.rightMargin: Style.space(8)
            spacing: Style.space(8)

            Column {
              width: parent.width - totpBadge.width - parent.spacing
              anchors.verticalCenter: parent.verticalCenter
              Text {
                width: parent.width
                elide: Text.ElideRight
                text: modelData.title
                color: root.contentForeground
                font.family: Style.font.family
                font.pixelSize: Style.font.body
              }
              Text {
                width: parent.width
                elide: Text.ElideRight
                visible: modelData.username !== ""
                text: modelData.username
                color: root.mutedForeground
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
              }
            }

            Text {
              id: totpBadge
              anchors.verticalCenter: parent.verticalCenter
              visible: modelData.has_totp
              width: visible ? implicitWidth : 0
              text: "TOTP"
              color: root.mutedForeground
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
            }
          }
        }

        Text {
          visible: root.filtered.length === 0 && root.entries.length > 0
          anchors.centerIn: parent
          text: "Sin resultados"
          color: root.mutedForeground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }

      Text {
        width: parent.width
        visible: root.statusText !== ""
        wrapMode: Text.WordWrap
        text: root.statusText
        color: root.pendingDelete ? Color.urgent : root.mutedForeground
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }

      Column {
        width: parent.width
        visible: !root.needsLogin
        Text {
          width: parent.width
          text: "↵ contraseña · ctrl+u usuario · ctrl+l URL · alt↵ TOTP"
          color: root.mutedForeground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
        Text {
          width: parent.width
          text: "shift↵ tipear · ctrl+e editar · ctrl+⌫ borrar · + crear"
          color: root.mutedForeground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }
    }
  }
}
