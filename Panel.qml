import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Panel de Lemonade con las tres pestañas de la web app:
//   🔑 Passwords · 🔒 Env Vault · 📝 Notas   (ctrl+1/2/3 o Tab para cambiar)
//
// Keyboard-first: escribir filtra, ↑/↓ navega, Enter actúa según la pestaña.
// El panel nunca ve un secreto: el CLI habla con el backend y entrega a
// wl-copy/wtype por stdin. La master password del Env Vault se pide en una
// terminal flotante — jamás en QML.
Panel {
  id: root
  moduleName: "io.github.oktubr3.lemonade"
  ipcTarget: moduleName
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property color contentForeground: bar ? bar.foreground : Color.foreground

  // Paleta "Lemon Noir" — el modo oscuro de la web app de Lemonade.
  readonly property color lemonGold: "#FFD700"
  readonly property color lemonGreen: "#28A745"
  readonly property color lemonRed: "#DC3545"
  readonly property color lemonSurface: "#161B22"
  readonly property color lemonElevated: "#21262D"
  readonly property color lemonBorder: "#30363D"
  readonly property color lemonText: "#F0F6FC"
  readonly property color lemonMuted: "#8B949E"

  // --- estado por pestaña ---
  property int currentTab: 0            // 0 passwords · 1 env · 2 notas
  property var passEntries: []
  property var noteEntries: []
  property var projEntries: []
  property var varEntries: []
  property int envLevel: 0              // 0 proyectos · 1 variables
  property var envProject: null         // {id,title} del proyecto abierto
  property bool notesLoaded: false
  property bool projsLoaded: false

  property var filtered: []
  property int selectedIndex: 0
  property string statusText: ""
  property bool needsLogin: false
  property bool busy: false
  property var pendingDelete: null

  readonly property var tabDefs: [
    { icon: "", label: "Passwords" },
    { icon: "󰌾", label: "Env Vault" },
    { icon: "", label: "Notas" }
  ]

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
      applyFilter()
    }
  }

  function records() {
    if (currentTab === 0) return passEntries
    if (currentTab === 2) return noteEntries
    return envLevel === 0 ? projEntries : varEntries
  }

  function applyFilter() {
    var data = records()
    var q = searchField.text.toLowerCase().trim()
    if (q === "") {
      filtered = data
    } else {
      var terms = q.split(/\s+/)
      filtered = data.filter(function(e) {
        var hay = ((e.title || "") + " " + (e.subtitle || "") + " " + (e.url || "")).toLowerCase()
        return terms.every(function(t) { return hay.indexOf(t) !== -1 })
      })
    }
    if (selectedIndex >= filtered.length) selectedIndex = Math.max(0, filtered.length - 1)
  }

  function switchTab(t) {
    if (t === currentTab) return
    cancelDelete()
    currentTab = t
    envLevel = 0
    envProject = null
    searchField.text = ""
    selectedIndex = 0
    statusText = ""
    if (t === 2 && !notesLoaded && !notesProc.running) notesProc.running = true
    if (t === 1 && !projsLoaded && !projsProc.running) projsProc.running = true
    applyFilter()
  }

  function current() {
    return filtered.length > 0 && selectedIndex < filtered.length ? filtered[selectedIndex] : null
  }

  function move(delta) {
    cancelDelete()
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

  function runInTerminal(cmd) {
    root.close()
    Util.execDetached("omarchy-launch-floating-terminal-with-presentation " +
      "\"" + cmd + "; echo; echo 'Enter para cerrar'; read -r\"")
  }

  // --- acción principal (Enter / clic) según pestaña ---
  function activate() {
    var e = current(); if (!e) return
    if (pendingDelete) { confirmDelete(); return }
    if (e.kind === "pass") {
      statusText = "Copiando contraseña de " + e.title + "…"
      runAction(["copy", e.id], false)
    } else if (e.kind === "note") {
      statusText = "Copiando nota " + e.title + "…"
      runAction(["note", "copy", e.id], false)
    } else if (e.kind === "proj") {
      envProject = e
      envLevel = 1
      varEntries = []
      searchField.text = ""
      selectedIndex = 0
      varsProc.projectId = e.id
      varsProc.running = true
      applyFilter()
    } else if (e.kind === "var") {
      // La master password se pide en la terminal, nunca acá.
      runInTerminal("lemonade env copy " + envProject.id + " " + e.id)
    }
  }

  // Ctrl+O: el ritual completo de la web en una tecla — copia la contraseña
  // y abre la URL en el browser por default. El ícono ↗ de la fila solo abre.
  function openUrl(withPassword) {
    var e = current(); if (!e || e.kind !== "pass") return
    if (!e.url || e.url === "") { statusText = e.title + " no tiene URL"; return }
    runAction(withPassword ? ["open", e.id, "--copy"] : ["open", e.id], true)
  }

  function copyUsername() {
    var e = current(); if (!e || e.kind !== "pass") return
    statusText = "Copiando usuario de " + e.title + "…"
    runAction(["copy", e.id, "--field", "username"], false)
  }

  function copyUrl() {
    var e = current(); if (!e || e.kind !== "pass") return
    if (!e.url || e.url === "") { statusText = e.title + " no tiene URL"; return }
    runAction(["copy", e.id, "--field", "url"], false)
  }

  function copyTotp() {
    var e = current(); if (!e || e.kind !== "pass") return
    if (!e.totp) { statusText = e.title + " no tiene TOTP"; return }
    runAction(["totp", e.id], false)
  }

  function toggleFav() {
    var e = current(); if (!e || e.kind !== "pass") return
    runAction(["fav", e.id], false)
  }

  function shareEntry() {
    var e = current(); if (!e) return
    if (e.kind === "pass") runAction(["share", e.id], false)
    else if (e.kind === "proj") runInTerminal("lemonade env export " + e.id)
  }

  function showDetail() {
    var e = current(); if (!e) return
    if (e.kind === "pass") runInTerminal("lemonade show " + e.id)
    else if (e.kind === "note") runInTerminal("lemonade note show " + e.id)
  }

  function editEntry() {
    var e = current(); if (!e) return
    if (e.kind === "pass") runInTerminal("lemonade edit " + e.id)
    else if (e.kind === "note") runInTerminal("lemonade note edit " + e.id)
  }

  function autotype() {
    var e = current(); if (!e || e.kind !== "pass") return
    runAction(["type", e.id, "--delay", "650"], true)
  }

  function addNew() {
    if (currentTab === 0) runInTerminal("lemonade add")
    else if (currentTab === 2) runInTerminal("lemonade note add")
  }

  function armDelete() {
    var e = current(); if (!e) return
    if (e.kind !== "pass" && e.kind !== "note") return
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
    if (e.kind === "note") runAction(["note", "rm", e.id, "--yes"], false)
    else runAction(["rm", e.id, "--yes"], false)
  }

  function goBack() {
    if (pendingDelete) { cancelDelete(); return }
    if (currentTab === 1 && envLevel === 1) {
      envLevel = 0
      envProject = null
      searchField.text = ""
      selectedIndex = 0
      applyFilter()
      return
    }
    root.close()
  }

  // --- procesos ---

  Process {
    id: listProc
    property bool refresh: false
    command: refresh ? ["lemonade", "list", "--json", "--refresh"]
                     : ["lemonade", "list", "--json"]
    stdout: StdioCollector { id: listOut; waitForEnd: true }
    stderr: StdioCollector { id: listErr; waitForEnd: true }
    onExited: function(exitCode) {
      if (exitCode === 0) {
        try {
          root.passEntries = JSON.parse(listOut.text).map(function(e) {
            return { kind: "pass", id: e.id, title: e.title, subtitle: e.username,
                     url: e.url, totp: e.has_totp, star: e.highlighted }
          })
          root.needsLogin = false
          root.applyFilter()
        } catch (e) {
          root.statusText = "Respuesta ilegible del CLI"
        }
      } else {
        var err = String(listErr.text).trim()
        if (err.indexOf("sin sesión") !== -1 || err.indexOf("login") !== -1) {
          root.needsLogin = true
        } else if (root.passEntries.length === 0) {
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
    id: notesProc
    command: ["lemonade", "note", "list", "--json"]
    stdout: StdioCollector { id: notesOut; waitForEnd: true }
    onExited: function(exitCode) {
      if (exitCode === 0) {
        try {
          root.noteEntries = JSON.parse(notesOut.text).map(function(n) {
            return { kind: "note", id: n.id, title: n.title, subtitle: "" }
          })
          root.notesLoaded = true
          root.applyFilter()
        } catch (e) { root.statusText = "No pude leer las notas" }
      } else {
        root.statusText = "lemonade note list falló"
      }
    }
  }

  Process {
    id: projsProc
    command: ["lemonade", "env", "projects", "--json"]
    stdout: StdioCollector { id: projsOut; waitForEnd: true }
    onExited: function(exitCode) {
      if (exitCode === 0) {
        try {
          root.projEntries = JSON.parse(projsOut.text).map(function(p) {
            return { kind: "proj", id: p.id, title: p.name, subtitle: "" }
          })
          root.projsLoaded = true
          root.applyFilter()
        } catch (e) { root.statusText = "No pude leer los proyectos" }
      } else {
        root.statusText = "lemonade env projects falló"
      }
    }
  }

  Process {
    id: varsProc
    property string projectId: ""
    command: ["lemonade", "env", "vars", projectId, "--json"]
    stdout: StdioCollector { id: varsOut; waitForEnd: true }
    onExited: function(exitCode) {
      if (exitCode === 0) {
        try {
          root.varEntries = JSON.parse(varsOut.text).map(function(v) {
            return { kind: "var", id: v.name, title: v.name, subtitle: "" }
          })
          root.applyFilter()
        } catch (e) { root.statusText = "No pude leer las variables" }
      } else {
        root.statusText = "lemonade env vars falló"
      }
    }
  }

  Process {
    id: actionProc
    stderr: StdioCollector { id: actionErr; waitForEnd: true }
    onExited: function(exitCode) {
      root.busy = false
      var msg = String(actionErr.text).trim()
      if (exitCode === 0) {
        root.statusText = msg !== "" ? msg : "Listo."
        statusClear.restart()
        if (!listProc.running) { listProc.refresh = false; listProc.running = true }
        if (root.currentTab === 2 && !notesProc.running) notesProc.running = true
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
    contentHeight: panel.fittedContentHeight(contentColumn.implicitHeight, Style.space(620))

    // Fondo "Lemon Noir" sobre el theme del shell.
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

      // Header: título bicolor + contador + FAB
      Item {
        width: parent.width
        height: Style.space(22)

        Row {
          spacing: Style.space(6)
          anchors.verticalCenter: parent.verticalCenter
          Text {
            text: ""
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
          visible: !root.needsLogin
          text: root.currentTab === 0 ? root.passEntries.length + " passwords"
              : root.currentTab === 2 ? root.noteEntries.length + " notas"
              : (root.envLevel === 0 ? root.projEntries.length + " proyectos"
                                     : root.varEntries.length + " variables")
          color: root.lemonMuted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
        }

        Rectangle {
          visible: !root.needsLogin && root.currentTab !== 1
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
            onClicked: root.addNew()
          }
        }
      }

      // Pestañas estilo web: ícono + label, subrayado dorado en la activa
      Row {
        width: parent.width
        spacing: Style.space(4)

        Repeater {
          model: root.tabDefs

          delegate: Rectangle {
            required property var modelData
            required property int index
            width: (contentColumn.width - Style.space(8)) / 3
            height: Style.space(26)
            radius: Style.space(5)
            color: tabArea.containsMouse && index !== root.currentTab ? root.lemonSurface : "transparent"

            Row {
              anchors.centerIn: parent
              spacing: Style.space(5)
              Text {
                text: modelData.icon
                color: index === root.currentTab ? root.lemonGold : root.lemonMuted
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                anchors.verticalCenter: parent.verticalCenter
              }
              Text {
                text: modelData.label
                color: index === root.currentTab ? root.lemonText : root.lemonMuted
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: index === root.currentTab
                anchors.verticalCenter: parent.verticalCenter
              }
            }

            Rectangle {
              anchors.bottom: parent.bottom
              anchors.horizontalCenter: parent.horizontalCenter
              width: parent.width * 0.7
              height: 2
              radius: 1
              color: index === root.currentTab ? root.lemonGold : "transparent"
            }

            MouseArea {
              id: tabArea
              anchors.fill: parent
              hoverEnabled: true
              onClicked: root.switchTab(index)
            }
          }
        }
      }

      // Breadcrumb del Env Vault cuando estás dentro de un proyecto
      Row {
        width: parent.width
        visible: root.currentTab === 1 && root.envLevel === 1
        spacing: Style.space(5)
        Text {
          text: "←"
          color: root.lemonGold
          font.family: Style.font.family
          font.pixelSize: Style.font.body
          MouseArea {
            anchors.fill: parent
            anchors.margins: -Style.space(4)
            onClicked: root.goBack()
          }
        }
        Text {
          text: root.envProject ? root.envProject.title : ""
          color: root.lemonText
          font.family: Style.font.family
          font.pixelSize: Style.font.body
          font.bold: true
        }
        Text {
          text: "(Esc vuelve)"
          color: root.lemonMuted
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          anchors.verticalCenter: parent.verticalCenter
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

        Keys.onUpPressed: root.move(-1)
        Keys.onDownPressed: root.move(1)
        Keys.onEscapePressed: root.goBack()
        Keys.onTabPressed: root.switchTab((root.currentTab + 1) % 3)
        Keys.onDeletePressed: function(event) {
          if (event.modifiers & Qt.ControlModifier) root.armDelete()
          else event.accepted = false
        }
        Keys.onPressed: function(event) {
          if (event.modifiers & Qt.ControlModifier) {
            if (event.key === Qt.Key_1) { root.switchTab(0); event.accepted = true; return }
            if (event.key === Qt.Key_2) { root.switchTab(1); event.accepted = true; return }
            if (event.key === Qt.Key_3) { root.switchTab(2); event.accepted = true; return }
            if (event.key === Qt.Key_E) { root.editEntry(); event.accepted = true; return }
            if (event.key === Qt.Key_U) { root.copyUsername(); event.accepted = true; return }
            if (event.key === Qt.Key_L) { root.copyUrl(); event.accepted = true; return }
            if (event.key === Qt.Key_F) { root.toggleFav(); event.accepted = true; return }
            if (event.key === Qt.Key_S) { root.shareEntry(); event.accepted = true; return }
            if (event.key === Qt.Key_D) { root.showDetail(); event.accepted = true; return }
            if (event.key === Qt.Key_O) { root.openUrl(true); event.accepted = true; return }
          }
        }
        function dispatchEnter(mods) {
          if (root.pendingDelete) { root.confirmDelete(); return }
          if (root.currentTab === 0) {
            if (mods & Qt.ShiftModifier) { root.autotype(); return }
            if (mods & Qt.ControlModifier) { root.copyUsername(); return }
            if (mods & Qt.AltModifier) { root.copyTotp(); return }
          }
          root.activate()
        }
        Keys.onReturnPressed: function(event) { dispatchEnter(event.modifiers) }
        Keys.onEnterPressed: function(event) { dispatchEnter(event.modifiers) }
      }

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
          height: modelData.subtitle && modelData.subtitle !== "" ? Style.space(36) : Style.space(30)
          radius: Style.space(6)
          color: index === root.selectedIndex ? root.lemonElevated : root.lemonSurface
          border.color: index === root.selectedIndex ? root.lemonGold : root.lemonBorder
          border.width: 1

          Rectangle {
            visible: modelData.star === true
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
              else root.activate()
            }
          }

          Row {
            anchors.fill: parent
            anchors.leftMargin: Style.space(8)
            anchors.rightMargin: Style.space(8)
            spacing: Style.space(8)

            Column {
              width: parent.width - linkBadge.width - kindBadge.width - totpBadge.width - starBadge.width - parent.spacing * 4
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
                visible: modelData.subtitle !== undefined && modelData.subtitle !== ""
                text: modelData.subtitle || ""
                color: root.lemonMuted
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
              }
            }

            Text {
              id: linkBadge
              anchors.verticalCenter: parent.verticalCenter
              visible: modelData.kind === "pass" && modelData.url !== undefined && modelData.url !== ""
              width: visible ? implicitWidth + Style.space(4) : 0
              text: "↗"
              color: linkArea.containsMouse ? root.lemonGold : root.lemonMuted
              font.family: Style.font.family
              font.pixelSize: Style.font.body
              MouseArea {
                id: linkArea
                anchors.fill: parent
                anchors.margins: -Style.space(3)
                hoverEnabled: true
                onClicked: function(mouse) {
                  root.selectedIndex = index
                  root.openUrl(false)
                  mouse.accepted = true
                }
              }
            }

            Text {
              id: kindBadge
              anchors.verticalCenter: parent.verticalCenter
              visible: modelData.kind === "proj"
              width: visible ? implicitWidth : 0
              text: "→"
              color: root.lemonMuted
              font.family: Style.font.family
              font.pixelSize: Style.font.body
            }

            Text {
              id: totpBadge
              anchors.verticalCenter: parent.verticalCenter
              visible: modelData.totp === true
              width: visible ? implicitWidth : 0
              text: "TOTP"
              color: root.lemonGold
              font.family: Style.font.family
              font.pixelSize: Style.font.bodySmall
            }

            Text {
              id: starBadge
              anchors.verticalCenter: parent.verticalCenter
              visible: modelData.star === true
              width: visible ? implicitWidth : 0
              text: "★"
              color: root.lemonGreen
              font.family: Style.font.family
              font.pixelSize: Style.font.body
            }
          }
        }

        Text {
          visible: root.filtered.length === 0 && !root.needsLogin
          anchors.centerIn: parent
          text: {
            if (root.currentTab === 2 && !root.notesLoaded) return "Cargando notas…"
            if (root.currentTab === 1 && root.envLevel === 0 && !root.projsLoaded) return "Cargando proyectos…"
            if (root.currentTab === 1 && root.envLevel === 1 && root.varEntries.length === 0) return "Cargando variables…"
            return root.records().length > 0 ? "Sin resultados" : "Vacío"
          }
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

      // Ayuda: chips de teclas según pestaña
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
            model: {
              var common = [{ k: "ctrl 1·2·3", l: "pestañas" }]
              if (root.currentTab === 0) return [
                { k: "↵", l: "contraseña" },
                { k: "ctrl u", l: "usuario" },
                { k: "ctrl l", l: "URL" },
                { k: "ctrl o", l: "abrir URL + copiar pass" },
                { k: "alt ↵", l: "TOTP" },
                { k: "ctrl s", l: "compartir" },
                { k: "ctrl f", l: "★" },
                { k: "shift ↵", l: "tipear" },
                { k: "ctrl e", l: "editar" },
                { k: "ctrl d", l: "detalle" },
                { k: "ctrl ⌫", l: "borrar" }
              ].concat(common)
              if (root.currentTab === 2) return [
                { k: "↵", l: "copiar nota" },
                { k: "ctrl d", l: "ver" },
                { k: "ctrl e", l: "editar" },
                { k: "ctrl ⌫", l: "borrar" },
                { k: "+", l: "crear" }
              ].concat(common)
              if (root.envLevel === 0) return [
                { k: "↵", l: "abrir proyecto" },
                { k: "ctrl s", l: "exportar .env" }
              ].concat(common)
              return [
                { k: "↵", l: "copiar valor (pide master password)" },
                { k: "esc", l: "volver" }
              ].concat(common)
            }

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
