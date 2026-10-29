/* global document, HTMLElement, HTMLInputElement, HTMLTextAreaElement, Event, TextEncoder, getComputedStyle, crypto */
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
    if (
      ["group", "navigation", "main", "tabpanel", "tablist", "dialog", "status", "alert"].includes(
        role(element),
      )
    )
      return clean(element.getAttribute("title"));
    return clean(clean(visibleText(element)) || element.getAttribute("title") || element.value);
  }
  // Targets identify node objects, not a position that another node may take.
  // Weak references and pruning keep only connected nodes of this document;
  // the document nonce makes a target from before a reload unusable afterward.
  const key = Symbol.for("orbit.testBridge.targets");
  const store = (globalThis[key] ??= {
    nonce: Array.from(crypto.getRandomValues(new Uint32Array(2))),
    ids: new WeakMap(),
    refs: new Map(),
    next: 1,
  });
  for (const [id, reference] of store.refs)
    if (!reference.deref()?.isConnected) store.refs.delete(id);
  function target(element) {
    if (!store.ids.has(element)) store.ids.set(element, store.next++);
    const id = store.ids.get(element);
    store.refs.set(id, new WeakRef(element));
    return [...store.nonce, id];
  }
  function resolve(token) {
    if (
      !Array.isArray(token) ||
      token.length !== 3 ||
      token[0] !== store.nonce[0] ||
      token[1] !== store.nonce[1]
    )
      return null;
    return store.refs.get(token[2])?.deref();
  }
  function semantic(element) {
    if (role(element) !== "group") return true;
    return (
      element.hasAttribute("aria-label") ||
      element.hasAttribute("aria-labelledby") ||
      element.hasAttribute("role")
    );
  }
  function axRole(element) {
    if (element.tagName === "TEXTAREA") return "AXTextArea";
    return (
      {
        button: "AXButton",
        textbox: "AXTextField",
        heading: "AXHeading",
        link: "AXLink",
        radio: "AXRadioButton",
        checkbox: "AXCheckBox",
        combobox: "AXPopUpButton",
        tab: "AXRadioButton",
      }[role(element)] || "AXGroup"
    );
  }
  function treeLine(row, depth, actionable) {
    const prefix = `${"  ".repeat(depth)}- ${actionable ? `[${row.index}] ` : ""}${row.ax_role}`;
    if (row.ax_role === "AXStaticText") return `${prefix} = "${row.text}"`;
    if (["AXTextField", "AXTextArea", "AXPopUpButton"].includes(row.ax_role))
      return `${prefix} = "${row.text}" (${row.name})`;
    if (row.ax_role === "AXGroup") return `${prefix}${row.name ? ` (${row.name})` : ""}`;
    if (row.ax_role === "AXHeading") return prefix;
    return `${prefix} "${row.name}"`;
  }
  function walk(element, depth, rows, lines) {
    if (!visible(element) || ["SCRIPT", "STYLE", "TEMPLATE"].includes(element.tagName)) return;
    const kept = semantic(element);
    const kind = role(element);
    const actionable = [
      "button",
      "link",
      "textbox",
      "radio",
      "checkbox",
      "combobox",
      "tab",
    ].includes(kind);
    if (kept) {
      const text = ["INPUT", "TEXTAREA"].includes(element.tagName)
        ? element.value
        : visibleText(element);
      const row = {
        role: kind,
        name: name(element),
        text: clean(text),
        ax_role: axRole(element),
        index: rows.length + 1,
        target: target(element),
      };
      if (element.tagName === "SELECT") row.text = clean(element.selectedOptions[0]?.textContent);
      rows.push(row);
      lines.push(treeLine(row, depth, actionable));
      // Controls expose their own name/value once. Links also keep their text
      // children: the meeting list identifies the title before its metadata.
      if (actionable && kind !== "link") return;
    }
    if (
      ["P", "TIME"].includes(element.tagName) &&
      !element.querySelector("button,a[href],input,textarea,select,[role]")
    ) {
      const text = clean(visibleText(element));
      if (text) {
        const row = {
          role: "text",
          name: "",
          text,
          ax_role: "AXStaticText",
          index: rows.length + 1,
        };
        rows.push(row);
        lines.push(treeLine(row, depth + Number(kept), false));
      }
      return;
    }
    const children = Array.from(element.childNodes);
    for (const child of children) {
      if (child instanceof HTMLElement) walk(child, depth + Number(kept), rows, lines);
      else if (child.nodeType === 3 && clean(child.textContent)) {
        const row = {
          role: "text",
          name: "",
          text: clean(child.textContent),
          ax_role: "AXStaticText",
          index: rows.length + 1,
        };
        rows.push(row);
        lines.push(treeLine(row, depth + Number(kept), false));
      }
    }
  }
  function read() {
    const rows = [],
      lines = [];
    for (const element of document.body.children)
      if (element instanceof HTMLElement) walk(element, 0, rows, lines);
    let kept = lines.map((_, index) => index);
    if (request.query) {
      const matches = new Set();
      lines.forEach((line, at) => {
        if (!line.toLowerCase().includes(request.query.toLowerCase())) return;
        matches.add(at);
        let indent = line.search(/\S/);
        for (let earlier = at - 1; earlier >= 0; earlier--) {
          const other = lines[earlier].search(/\S/);
          if (other < indent) {
            matches.add(earlier);
            indent = other;
          }
        }
      });
      kept = kept.filter((index) => matches.has(index));
    }
    return {
      ok: true,
      elements: kept.map((index) => rows[index]),
      tree_markdown: kept.map((index) => lines[index]).join("\n") + "\n",
    };
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
      if (request.command === "read") return read();
      const scope = request.target
        ? [resolve(request.target)].filter((element) => elements.includes(element))
        : elements;
      const matches = scope.filter(
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
