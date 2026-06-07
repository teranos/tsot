// Main-thread JS bridge for the Elm dev-tool port. Owns whatever Web
// Platform surface the Elm app can't touch directly via ports — the
// wasm Worker handle, IndexedDB, SharedArrayBuffer atomic writes, file
// download, prompt/confirm, setInterval.
//
// H7-Elm Stage 1: just boots the Elm app into <div id="elm-root">.
// Stage 2 and on, each port the Elm side declares gets its handler
// added here (one function per `app.ports.X.subscribe` for Elm→JS
// outbound, one `app.ports.X.send(...)` call per JS→Elm inbound).
(function () {
  var node = document.getElementById('elm-root');
  if (!node) {
    console.error('js-bridge: <div id="elm-root"> missing from play.html');
    return;
  }
  if (typeof Elm === 'undefined' || !Elm.Main || typeof Elm.Main.init !== 'function') {
    console.error('js-bridge: Elm.Main missing — did bundle.js load before js-bridge.js?');
    return;
  }
  Elm.Main.init({ node: node });
})();
