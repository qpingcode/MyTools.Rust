"use strict";

const byId = (id) => document.getElementById(id);
let labels = {};
const label = (key) => labels[key] ?? key;
let feedbackTimer;
let listVersion = 0;
let lastItems = "";

function feedback(message, durable = false) {
  const status = byId("status");
  clearTimeout(feedbackTimer);
  status.textContent = message;
  status.hidden = false;
  if (!durable) feedbackTimer = setTimeout(() => { status.hidden = true; }, 5000);
}

async function request(path, options) {
  let response;
  try {
    response = await fetch(path, options);
  } catch {
    throw new Error(label("web.disconnected"));
  }
  if (!response.ok) {
    const reason = { 413: "too_large", 429: "busy", 507: "full" }[response.status] ?? "failed";
    throw new Error(label("web." + reason));
  }
  return response;
}

function copyText(text, source) {
  const field = document.createElement("textarea");
  field.value = text;
  field.readOnly = true;
  field.className = "clipboard-field";
  document.body.append(field);
  field.select();
  field.setSelectionRange(0, text.length);
  let copied = false;
  try {
    copied = document.execCommand("copy");
  } catch {
    // Some mobile browsers restrict clipboard access on HTTP pages.
  }
  field.remove();
  source.focus();
  if (copied) {
    feedback(label("web.copied"));
    return;
  }
  window.prompt(label("web.copy_failed"), text);
}

function button(key, action, className = "") {
  const control = document.createElement("button");
  control.type = "button";
  control.textContent = label(key);
  control.className = className;
  control.addEventListener("click", action);
  return control;
}

function renderItem(item) {
  const row = document.createElement("li");
  row.className = "item";
  const details = document.createElement("div");
  details.className = "details";
  const isText = item.text_preview !== null;
  const title = document.createElement("strong");
  title.textContent = isText ? label("web.message") : item.filename;
  details.append(title);
  if (isText) {
    const preview = document.createElement("p");
    preview.className = "preview";
    preview.textContent = item.text_preview;
    details.append(preview);
  }
  const size = document.createElement("p");
  size.className = "size";
  const units = item.size >= 1048576 ? [1048576, "megabyte"] :
    item.size >= 1024 ? [1024, "kilobyte"] : [1, "byte"];
  size.textContent = new Intl.NumberFormat(document.documentElement.lang, {
    style: "unit", unit: units[1], maximumFractionDigits: 1
  }).format(item.size / units[0]);
  details.append(size);

  const path = "items/" + item.id;
  const actions = document.createElement("div");
  actions.className = "actions";
  if (isText) {
    actions.append(button("web.copy", async (event) => {
      const source = event.currentTarget;
      try {
        const response = await request(path);
        copyText(await response.text(), source);
      } catch (error) {
        feedback(error.message);
      }
    }));
  }
  const download = document.createElement("a");
  download.className = "download";
  download.href = path;
  download.download = item.filename;
  download.textContent = label("web.download");
  actions.append(download, button("web.delete", async () => {
    if (!window.confirm(label("web.confirm_delete"))) return;
    try {
      await request(path, { method: "DELETE" });
      await refresh();
    } catch (error) {
      feedback(error.message);
    }
  }, "remove"));
  row.append(details, actions);
  return row;
}

async function refresh() {
  const version = ++listVersion;
  const status = byId("connection-status");
  try {
    const response = await request("items", { signal: AbortSignal.timeout(4500) });
    const body = await response.text();
    if (version !== listVersion) return;
    status.hidden = true;
    if (body === lastItems) return;
    const items = JSON.parse(body);
    byId("items").replaceChildren(...items.map(renderItem));
    byId("empty").hidden = items.length !== 0;
    lastItems = body;
  } catch {
    if (version !== listVersion) return;
    status.textContent = label("web.disconnected");
    status.hidden = false;
  }
}

function configureLinks(links) {
  byId("no-network").hidden = links.length !== 0;
  if (!links.length) return;
  byId("connection").hidden = false;
  const network = byId("network");
  for (const link of links) network.add(new Option(link.ip, link.ip));
  if (links.some((link) => link.ip === location.hostname)) network.value = location.hostname;
  const address = byId("address");
  function selectLink() {
    const link = links.find((choice) => choice.ip === network.value);
    address.href = link.url;
    address.textContent = link.url;
    byId("qr").src = "qr/" + link.ip;
  }
  network.addEventListener("change", selectLink);
  byId("copy-link").addEventListener("click", (event) => copyText(address.href, event.currentTarget));
  selectLink();
}

function bindForm(id, upload) {
  byId(id).addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const controls = [...form.elements];
    controls.forEach((control) => { control.disabled = true; });
    feedback(label("web.sending"), true);
    try {
      await upload();
      form.reset();
      feedback(label("web.sent"));
    } catch (error) {
      feedback(error.message);
    } finally {
      controls.forEach((control) => { control.disabled = false; });
      await refresh();
    }
  });
}

async function initialize() {
  const response = await fetch("session");
  if (!response.ok) return;
  const session = await response.json();
  labels = { ...JSON.parse(session.english), ...JSON.parse(session.localized) };
  document.documentElement.lang = session.language;
  document.title = label("name");
  document.body.classList.toggle("local", location.hostname === "127.0.0.1");
  document.querySelectorAll("[data-label]").forEach((element) => {
    element.textContent = label(element.dataset.label);
  });
  configureLinks(session.links);
  bindForm("text-form", () => {
    const body = new TextEncoder().encode(byId("message").value);
    if (body.length > 64 * 1024) throw new Error(label("web.too_large"));
    return request("items", { method: "POST", headers: { "X-Transfer-Text": "1" }, body });
  });
  bindForm("file-form", async () => {
    const input = byId("files");
    const files = Array.from(input.files);
    if (files.some((file) => file.size > 64 * 1024 * 1024)) throw new Error(label("web.too_large"));
    let uploaded = 0;
    try {
      for (const file of files) {
        await request("items", {
          method: "POST", headers: { "X-Transfer-Filename": encodeURIComponent(file.name) }, body: file
        });
        uploaded++;
      }
    } catch (error) {
      if (!uploaded) throw error;
      const remaining = new DataTransfer();
      files.slice(uploaded).forEach((file) => remaining.items.add(file));
      input.files = remaining.files;
      throw new Error(label("web.partial").replace("{error}", () => error.message));
    }
  });
  byId("workspace").hidden = false;
  await refresh();
  setInterval(() => { if (!document.hidden) refresh(); }, 5000);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) refresh(); });
}

initialize();
