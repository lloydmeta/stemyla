
export function port_listen(port, on_message, on_error) {
  port.onmessage = (e) => on_message(e.data);
  port.onerror = (e) => on_error((e && e.message) || "its script did not load");
  port.onmessageerror = () => on_error("a message to the page could not be read");
}
export function port_post(port, msg, transfer) { port.postMessage(msg, transfer); }
export function port_end(port) { if (typeof port.terminate === "function") port.terminate(); }
export function worker_port(url) { return new Worker(url, { type: "module" }); }
