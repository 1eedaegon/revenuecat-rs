// revenuecat-rs demo frontend: drives the SDK entirely through Tauri IPC.

const invoke = window.__TAURI__.core.invoke;

const el = (id) => document.getElementById(id);
const packagesEl = el("packages");
const stampEl = el("entitlement-stamp");
const linesEl = el("lines");
const logEl = el("log");

function log(message, isError = false) {
  const item = document.createElement("li");
  const time = document.createElement("span");
  time.className = "t";
  time.textContent = new Date().toLocaleTimeString();
  const text = document.createElement("span");
  if (isError) text.className = "err";
  text.textContent = message;
  item.append(time, text);
  logEl.prepend(item);
}

async function call(cmd, args = {}) {
  try {
    const result = await invoke(cmd, args);
    log(`${cmd} ✓`);
    return result;
  } catch (error) {
    const message = typeof error === "object" ? `${error.code}: ${error.message}` : String(error);
    log(`${cmd} ✗ ${message}`, true);
    throw error;
  }
}

function renderSession(session) {
  const chip = el("user-chip");
  chip.textContent = session.app_user_id;
  chip.title = session.app_user_id;
  chip.classList.toggle("identified", !session.is_anonymous);
  el("login-btn").hidden = !session.is_anonymous;
  el("logout-btn").hidden = session.is_anonymous;
}

function packageLabel(type) {
  return typeof type === "string" ? type : (type.Custom ?? "Package");
}

function renderOfferings(offerings) {
  packagesEl.replaceChildren();
  const current = offerings.current_offering_id
    ? offerings.all[offerings.current_offering_id]
    : null;
  if (!current) return;

  for (const pkg of current.packages) {
    const item = document.createElement("li");
    item.className = "package";

    const label = document.createElement("div");
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = pkg.store_product.title;
    const meta = document.createElement("span");
    meta.className = "meta";
    const period = pkg.store_product.subscription_period ?? "one-time";
    meta.textContent = `${packageLabel(pkg.package_type)} · ${pkg.store_product.identifier} · ${period}`;
    label.append(name, meta);

    const price = document.createElement("span");
    price.className = "price";
    price.textContent = formatPrice(pkg.store_product.price);

    const buy = document.createElement("button");
    buy.className = "primary";
    buy.textContent = "Buy";
    buy.addEventListener("click", async () => {
      buy.disabled = true;
      try {
        const result = await call("purchase", { packageId: pkg.identifier });
        renderCustomer(result.customer_info);
      } finally {
        buy.disabled = false;
      }
    });

    item.append(label, price, buy);
    packagesEl.append(item);
  }
}

function formatPrice(price) {
  const amount = price.amount_micros / 1_000_000;
  try {
    return new Intl.NumberFormat(undefined, { style: "currency", currency: price.currency })
      .format(amount);
  } catch {
    return `${amount.toFixed(2)} ${price.currency}`;
  }
}

function renderCustomer(info) {
  const active = Object.values(info.entitlements.all).filter((e) => e.is_active);
  stampEl.textContent = active.length
    ? active.map((e) => e.identifier.toUpperCase()).join(" · ")
    : "NO ENTITLEMENT";
  stampEl.classList.toggle("active", active.length > 0);
  stampEl.classList.toggle("none", active.length === 0);

  el("fact-user").textContent = info.original_app_user_id;
  const subs = Object.entries(info.subscriptions).filter(([, s]) => s.is_active);
  el("fact-subs").textContent = subs.length ? subs.map(([id]) => id).join(", ") : "none";
  const expiries = subs.map(([, s]) => s.expires_date).filter(Boolean).sort();
  el("fact-expiry").textContent = expiries.at(-1) ?? "–";

  linesEl.replaceChildren();
  const entries = [
    ...Object.entries(info.subscriptions).map(([id, s]) => ({
      id,
      date: s.purchase_date,
      note: s.is_active ? "active" : "expired",
      ok: s.is_active,
    })),
    ...info.non_subscription_transactions.map((t) => ({
      id: t.product_identifier,
      date: t.purchase_date,
      note: "one-time",
      ok: true,
    })),
  ];
  if (!entries.length) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "No purchases yet.";
    linesEl.append(empty);
    return;
  }
  for (const entry of entries) {
    const item = document.createElement("li");
    const left = document.createElement("span");
    left.textContent = `${entry.id}  ${entry.date?.slice(0, 10) ?? ""}`;
    const right = document.createElement("span");
    right.className = entry.ok ? "ok" : "";
    right.textContent = entry.note;
    item.append(left, right);
    linesEl.append(item);
  }
}

async function refreshAll() {
  renderSession(await call("session_info"));
  renderOfferings(await call("get_offerings"));
  renderCustomer(await call("get_customer_info"));
}

el("restore-btn").addEventListener("click", async () => {
  renderCustomer(await call("restore"));
});

const dialog = el("login-dialog");
el("login-btn").addEventListener("click", () => {
  el("login-id").value = "";
  dialog.showModal();
});
dialog.addEventListener("close", async () => {
  if (dialog.returnValue !== "ok") return;
  const id = el("login-id").value.trim();
  if (!id) return;
  const result = await call("login", { appUserId: id });
  log(result.created ? `user '${id}' created` : `user '${id}' existed — aliased`);
  renderSession(await call("session_info"));
  renderCustomer(result.customer_info);
});

el("logout-btn").addEventListener("click", async () => {
  renderCustomer(await call("logout"));
  renderSession(await call("session_info"));
});

refreshAll().then(() => log("SDK ready — Test Store via embedded mock backend"));
