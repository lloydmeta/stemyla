// Records every AudioContext the page creates in window.__e2e_ctxs.
(() => {
  const Real = window.AudioContext;
  if (!Real) return;
  window.__e2e_ctxs = [];
  window.AudioContext = function (...args) {
    const ctx = new Real(...args);
    window.__e2e_ctxs.push(ctx);
    return ctx;
  };
  window.AudioContext.prototype = Real.prototype;
})();
