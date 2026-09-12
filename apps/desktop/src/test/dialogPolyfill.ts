// jsdom has no browser top layer. This only models open/closed state for DOM tests;
// focus containment, Escape and viewport bounds are verified in headless Chromium.
if (typeof HTMLDialogElement !== "undefined") {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute("open"); };
}
