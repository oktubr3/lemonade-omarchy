import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Ícono de Lemonade en la barra. Clic (o IPC toggle) abre el panel de
// búsqueda. Toda la lógica de datos vive en el CLI `lemonade`; este QML
// solo dibuja y orquesta.
BarWidget {
  id: root
  moduleName: "io.github.oktubr3.lemonade"

  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false

  function injectPanel() {
    var target = panelLoader.item
    if (!target) return
    if ("bar" in target) target.bar = root.bar
    if ("settings" in target) target.settings = root.settings
    if ("anchorItem" in target) target.anchorItem = button
    if ("hostWidget" in target) target.hostWidget = root
  }

  function open() { if (panelLoader.item) panelLoader.item.open() }
  function close() { if (panelLoader.item) panelLoader.item.close() }
  function togglePanel() { if (panelLoader.item) panelLoader.item.toggle() }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onBarChanged: injectPanel()
  onSettingsChanged: injectPanel()

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: {
      root.injectPanel()
      Qt.callLater(root.injectPanel)
    }
  }

  IpcHandler {
    target: root.moduleName

    function toggle(): void { root.togglePanel() }
    function open(): void { root.open() }
    function close(): void { root.close() }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "\udb80\udf3e" // U+F033E nf-md-lock
    tooltipText: "Lemonade — contraseñas"
    onPressed: function(mb) {
      if (mb === Qt.LeftButton) root.togglePanel()
    }
  }
}
