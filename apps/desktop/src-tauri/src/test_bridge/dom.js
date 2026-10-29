/* global document, HTMLElement, HTMLInputElement, HTMLTextAreaElement, Event, TextEncoder, getComputedStyle */
/* eslint @typescript-eslint/no-unused-expressions: off -- this function expression is evaluated by Rust */
// Injected only by the Pruebas Rust feature. It returns data through Tauri's
// eval callback; it neither imports application commands nor invokes Tauri IPC.
(request) => {
  const refusal = (cause, details = {}) => ({ ok: false, cause, ...details });
  const clean = (text) => (text || "").replace(/\s+/g, " ").trim();
  function visible(element) {
    if (element.closest('[hidden], [aria-hidden="true"], [inert]')) return false;
    for (let parent = element; parent; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        style.visibility === "collapse"
      )
        return false;
    }
    return element.getClientRects().length > 0;
  }
  function role(element) {
    const explicit = element.getAttribute("role");
    if (explicit) return explicit.split(" ")[0];
    const tag = element.tagName.toLowerCase();
    if (tag === "input") {
      if (["button", "submit", "reset"].includes(element.type)) return "button";
      if (["checkbox", "radio"].includes(element.type)) return element.type;
      return "textbox";
    }
    if (tag === "a" && element.hasAttribute("href")) return "link";
    if (/^h[1-6]$/.test(tag)) return "heading";
    return (
      {
        button: "button",
        textarea: "textbox",
        select: "combobox",
        option: "option",
        output: "status",
        img: "img",
        nav: "navigation",
        main: "main",
        dialog: "dialog",
      }[tag] || "group"
    );
  }
  function visibleText(element) {
    return Array.from(element.childNodes)
      .map((child) => {
        if (child.nodeType === 3) return child.textContent;
        return child instanceof HTMLElement && visible(child) ? visibleText(child) : "";
      })
      .join("");
  }
  function name(element) {
    const ids = element.getAttribute("aria-labelledby");
    if (ids)
      return clean(
        ids
          .split(/\s+/)
          .map((id) => document.getElementById(id)?.textContent || "")
          .join(" "),
      );
    if (element.hasAttribute("aria-label")) return clean(element.getAttribute("aria-label"));
    if (element.labels?.length)
      return clean(
        Array.from(element.labels)
          .map((label) => label.textContent)
          .join(" "),
      );
    if (element.tagName === "IMG") return clean(element.getAttribute("alt"));
    if (["textbox", "combobox"].includes(role(element)))
      return clean(element.getAttribute("title") || element.getAttribute("placeholder"));
    return clean(clean(visibleText(element)) || element.getAttribute("title") || element.value);
  }
  function read(elements) {
    const rows = elements.map((element) => ({
      role: role(element),
      name: name(element),
      text: clean(
        ["INPUT", "TEXTAREA", "SELECT"].includes(element.tagName)
          ? element.value
          : element.textContent,
      ),
    }));
    const ax = {
      button: "AXButton",
      textbox: "AXTextField",
      heading: "AXHeading",
      link: "AXLink",
      radio: "AXRadioButton",
      checkbox: "AXCheckBox",
      combobox: "AXPopUpButton",
      group: "AXGroup",
    };
    const tree_markdown = rows
      .map((row) => `- ${ax[row.role] || "AXStaticText"} "${row.name}" = "${row.text}"`)
      .join("\n");
    return { ok: true, elements: rows, tree_markdown };
  }
  function type(element) {
    if (element.readOnly) return refusal("field_read_only", { name: request.name });
    const prototype =
      element instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
    if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) || !setter)
      return refusal("field_not_writable");
    element.focus();
    setter.call(element, request.text);
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true, count: 1 };
  }
  function execute() {
    try {
      // A queued evaluation may run after its Rust waiter expired. Refuse before
      // any effect; a lost callback still means an uncertain outcome, never retry.
      if (Date.now() >= request.deadline) return refusal("view_deadline_expired");
      const elements = Array.from(document.body.querySelectorAll("*")).filter(
        (element) =>
          element instanceof HTMLElement &&
          !["SCRIPT", "STYLE", "TEMPLATE"].includes(element.tagName) &&
          visible(element),
      );
      if (request.command === "read") return read(elements);
      const matches = elements.filter(
        (element) => role(element) === request.role && name(element) === request.name,
      );
      if (matches.length !== 1)
        return refusal("match_count", {
          role: request.role,
          name: request.name,
          count: matches.length,
        });
      if (Date.now() >= request.deadline) return refusal("view_deadline_expired");
      const element = matches[0];
      if (element.matches(":disabled, [aria-disabled='true']"))
        return refusal("element_disabled", { name: request.name });
      if (request.command === "type") return type(element);
      if (request.command !== "press") return refusal("unsupported_command");
      element.click();
      return { ok: true, count: 1 };
    } catch (error) {
      return refusal("dom_evaluation_failed", { detail: String(error) });
    }
  }
  const reply = execute();
  const bytes = new TextEncoder().encode(JSON.stringify(reply)).length;
  return bytes > request.max_reply_bytes
    ? refusal("reply_too_large", { bytes, max_bytes: request.max_reply_bytes })
    : reply;
};
