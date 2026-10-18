// Makes Chromium's Web Audio behave like Safari's: every AudioContext
// starts suspended, and resume() works only during a user activation
// (Chromium would otherwise start a suspended context by itself on the
// first click, hiding a page that never calls resume()).
(() => {
  const Real = window.AudioContext;
  if (!Real) return;
  window.AudioContext = function (...args) {
    const ctx = new Real(...args);
    ctx.suspend();
    const resume = ctx.resume.bind(ctx);
    ctx.resume = () => (navigator.userActivation.isActive ? resume() : Promise.resolve());
    return ctx;
  };
  window.AudioContext.prototype = Real.prototype;
})();
