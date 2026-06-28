/* digstore dev — injected DEV helpers (NOT part of your deployed capsule).
 *
 * 1. A minimal dev `window.chia` provider so wallet flows can be built and tested
 *    without a real wallet. It is a STUB: it returns plausible dev values and
 *    never touches mainnet. In production a real wallet injects the genuine
 *    provider, so do not rely on these exact return values.
 * 2. A live-reload poller that refreshes the page when `digstore dev` rebuilds.
 *
 * Both are injected at request time into HTML responses only.
 */
(function () {
  "use strict";

  // ---- Dev window.chia shim -------------------------------------------------
  if (!window.chia) {
    var listeners = {};
    window.chia = {
      isDigDev: true,
      // CHIP-0002-style connect/request surface (dev stub).
      connect: function () {
        return Promise.resolve(true);
      },
      request: function (args) {
        var method = args && args.method;
        switch (method) {
          case "chainId":
            return Promise.resolve("mainnet");
          case "connect":
            return Promise.resolve(true);
          case "getPublicKeys":
            return Promise.resolve([]);
          case "walletSignMessage":
          case "signMessageByAddress":
            return Promise.resolve({
              signature: "00".repeat(96),
              publicKey: "00".repeat(48),
              dev: true,
            });
          default:
            return Promise.reject(
              new Error(
                "[digstore dev] window.chia method '" +
                  method +
                  "' is not stubbed. A real wallet implements it in production.",
              ),
            );
        }
      },
      on: function (event, cb) {
        (listeners[event] = listeners[event] || []).push(cb);
      },
      removeListener: function (event, cb) {
        var l = listeners[event] || [];
        var i = l.indexOf(cb);
        if (i >= 0) l.splice(i, 1);
      },
    };
    console.info(
      "%c[digstore dev]%c injected a dev window.chia shim — wallet calls return dev values, nothing is signed on mainnet.",
      "color:#5eead4;font-weight:bold",
      "color:inherit",
    );
  }

  // ---- Live reload ----------------------------------------------------------
  var current = null;
  function poll() {
    fetch("/__dig/reload", { cache: "no-store" })
      .then(function (r) {
        return r.text();
      })
      .then(function (v) {
        if (current === null) {
          current = v;
        } else if (v !== current) {
          location.reload();
        }
      })
      .catch(function () {
        /* server gone or restarting — ignore until next tick */
      })
      .finally(function () {
        setTimeout(poll, 1000);
      });
  }
  poll();
})();
