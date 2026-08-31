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

  // Paleta "Lemon Noir" — el modo oscuro de la web app de Lemonade
  // (src/css/quasar.variables.scss). Acentos sobre el fondo del shell.
  readonly property color lemonGold: "#FFD700"
  readonly property color lemonGreen: "#28A745"
  readonly property color lemonRed: "#DC3545"
  readonly property color lemonSurface: "#161B22"
  readonly property color lemonElevated: "#21262D"
  readonly property color lemonBorder: "#30363D"
  readonly property color lemonText: "#F0F6FC"
  readonly property color lemonMuted: "#8B949E"

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

  function toggleFav() {
    var e = current(); if (!e) return
    statusText = (e.highlighted ? "Quitando" : "Marcando") + " favorita…"
    runAction(["fav", e.id], false)
  }

  function shareEntry() {
    var e = current(); if (!e) return
    statusText = "Preparando \"" + e.title + "\" para compartir…"
    runAction(["share", e.id], false)
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

  function showDetail() {
    var e = current(); if (!e) return
    root.close()
    Util.execDetached("omarchy-launch-floating-terminal-with-presentation " +
      "\"lemonade show " + e.id + "; echo; echo 'Enter para cerrar'; read -r\"")
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
    contentHeight: panel.fittedContentHeight(contentColumn.implicitHeight, Style.space(600))

    // Fondo "Lemon Noir": pinta el card completo con el dark-page de la
    // web app, por encima del fondo del theme del shell (el borde del
    // card sigue siendo del theme, para no perder la identidad Omarchy).
    Rectangle {
      anchors.fill: parent
      anchors.margins: -panel.padding
      radius: Style.cornerRadius
      color: "#0D1117"
    }

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
            color: root.lemonGold
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
            anchors.verticalCenter: parent.verticalCenter
          }
          Text {
            text: "Lemonade"
            color: root.lemonText
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
            font.bold: true
            anchors.verticalCenter: parent.verticalCenter
          }
          Text {
            text: "Password Manager"
            color: root.lemonGold
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
            font.bold: true
            anchors.verticalCenter: parent.verticalCenter
          }
        }

        Text {
          anchors.right: parent.right
          anchors.rightMargin: Style.space(30)
          anchors.verticalCenter: parent.verticalCenter
          visible: !root.needsLogin && root.entries.length > 0
          text: root.entries.length + " passwords"
          color: root.lemonMuted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }

        // "+": alta interactiva en terminal flotante (la contraseña se
        // ingresa oculta ahí; el panel jamás la ve).
        Rectangle {
          visible: !root.needsLogin
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(22)
          height: Style.space(22)
          radius: width / 2
          color: root.lemonGold
          opacity: addArea.containsMouse ? 1.0 : 0.88

          Text {
            anchors.centerIn: parent
            anchors.verticalCenterOffset: -1
            text: "+"
            color: "#0D1117"
            font.family: Style.font.family
            font.pixelSize: Style.font.heading
            font.bold: true
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
        accent: root.lemonGold
        foreground: root.lemonText
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
          else if (event.key === Qt.Key_F) { root.toggleFav(); event.accepted = true }
          else if (event.key === Qt.Key_S) { root.shareEntry(); event.accepted = true }
          else if (event.key === Qt.Key_D) { root.showDetail(); event.accepted = true }
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
          color: root.lemonText
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }

      ListView {
        id: entryList
        width: parent.width
        spacing: Style.space(4)
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
          height: Style.space(36)
          radius: Style.space(6)
          color: index === root.selectedIndex ? root.lemonElevated : root.lemonSurface
          border.color: index === root.selectedIndex ? root.lemonGold : root.lemonBorder
          border.width: 1

          // Borde izquierdo verde de las favoritas, como la web app.
          Rectangle {
            visible: modelData.highlighted === true
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            anchors.margins: 1
            width: Style.space(3)
            radius: width / 2
            color: root.lemonGreen
          }

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
              width: parent.width - starBadge.width - totpBadge.width - parent.spacing * 2
              anchors.verticalCenter: parent.verticalCenter
              Text {
                width: parent.width
                elide: Text.ElideRight
                text: modelData.title
                color: root.lemonText
                font.family: Style.font.family
                font.pixelSize: Style.font.body
              }
              Text {
                width: parent.width
                elide: Text.ElideRight
                visible: modelData.username !== ""
                text: modelData.username
                color: root.lemonMuted
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
              color: root.lemonGold
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
            }

            Text {
              id: starBadge
              anchors.verticalCenter: parent.verticalCenter
              visible: modelData.highlighted === true
              width: visible ? implicitWidth : 0
              text: "★"
              color: root.lemonGreen
              font.family: Style.font.family
              font.pixelSize: Style.font.body
            }
          }
        }

        Text {
          visible: root.filtered.length === 0 && root.entries.length > 0
          anchors.centerIn: parent
          text: "Sin resultados"
          color: root.lemonMuted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }
      }

      Text {
        width: parent.width
        visible: root.statusText !== ""
        wrapMode: Text.WordWrap
        text: root.statusText
        color: root.pendingDelete ? root.lemonRed : root.lemonMuted
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }

      // Ayuda: chips de teclas estilo footer, con wrap automático.
      Column {
        width: parent.width
        visible: !root.needsLogin
        spacing: Style.space(6)

        Rectangle {
          width: parent.width
          height: 1
          color: root.lemonBorder
        }

        Flow {
          width: parent.width
          spacing: Style.space(8)

          Repeater {
            model: [
              { k: "↵", l: "contraseña" },
              { k: "ctrl u", l: "usuario" },
              { k: "ctrl l", l: "URL" },
              { k: "alt ↵", l: "TOTP" },
              { k: "ctrl s", l: "compartir" },
              { k: "ctrl f", l: "★" },
              { k: "shift ↵", l: "tipear" },
              { k: "ctrl e", l: "editar" },
              { k: "ctrl d", l: "detalle" },
              { k: "ctrl ⌫", l: "borrar" }
            ]

            delegate: Row {
              required property var modelData
              spacing: Style.space(4)

              Rectangle {
                width: keyText.implicitWidth + Style.space(10)
                height: keyText.implicitHeight + Style.space(4)
                radius: Style.space(4)
                color: root.lemonElevated
                border.color: root.lemonBorder
                border.width: 1
                anchors.verticalCenter: parent.verticalCenter

                Text {
                  id: keyText
                  anchors.centerIn: parent
                  text: modelData.k
                  color: root.lemonGold
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                }
              }

              Text {
                anchors.verticalCenter: parent.verticalCenter
                text: modelData.l
                color: root.lemonMuted
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
              }
            }
          }
        }
      }
    }
  }
}
