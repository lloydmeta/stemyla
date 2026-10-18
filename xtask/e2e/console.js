// Keeps every console message and uncaught error of the page in
// window.__e2e_log, which `xtask e2e` reads and empties over WebDriver
// (classic WebDriver has no console events).  Served by `xtask serve
// --inject`, so it runs before the app.
(() => {
  const log = (window.__e2e_log = []);
  const text = (a) => {
    if (typeof a === "string") return a;
    if (a instanceof Error) return `${a.name}: ${a.message}`;
    try { return JSON.stringify(a); } catch (_) { return String(a); }
  };
  for (const level of ["log", "info", "warn", "error", "debug"]) {
    const real = console[level].bind(console);
    console[level] = (...args) => {
      log.push(`console.${level}: ${args.map(text).join(" ")}`);
      real(...args);
    };
  }
  addEventListener("error", (e) => log.push(`PAGE ERROR: ${e.message}`));
  addEventListener("unhandledrejection", (e) => log.push(`PAGE ERROR: unhandled rejection: ${text(e.reason)}`));
})();
