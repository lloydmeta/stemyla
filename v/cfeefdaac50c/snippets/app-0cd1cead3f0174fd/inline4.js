
// A macrotask: a message to a channel of its own.  Browsers clamp nested
// setTimeout calls to 4 ms, but not messages.
export function st_yield() {
  return new Promise((resolve) => {
    const c = new MessageChannel();
    c.port1.onmessage = () => { c.port1.close(); resolve(); };
    c.port2.postMessage(null);
  });
}
// The page's clock in milliseconds, in a window or in a worker.
export function st_now() { return performance.now(); }
