//! The scripts injected into the application page. Each one is a self-contained IIFE: the shared
//! helpers in [`LIB`] are prepended and `__ARGS__` is replaced with a JSON literal. Scripts only read
//! the page, tag fields with `data-jp-id`, set values and report back; they never navigate and never
//! submit anything.

/// Shared helpers: visibility, label discovery, option labels.
const LIB: &str = r#"
const norm = (s) => String(s == null ? '' : s).replace(/\s+/g, ' ').trim();
const shown = (el) => {
  if (!el || !el.isConnected) return false;
  for (let e = el; e && e.nodeType === 1; e = e.parentElement) {
    if (e.hidden) return false;
    if (getComputedStyle(e).display === 'none') return false;
  }
  return getComputedStyle(el).visibility !== 'hidden';
};
const rootOf = (sels) => {
  for (const s of sels) {
    let el = null;
    try { el = [...document.querySelectorAll(s)].find(shown); } catch (e) {}
    if (el) return el;
  }
  return null;
};
const textOf = (n) => (n ? norm(n.innerText !== undefined ? n.innerText : n.textContent) : '');
const cleanLabel = (s) =>
  norm(String(s).replace(/[*✱]/g, ' ').replace(/\(required\)/ig, ' ').replace(/\brequired\s*$/i, ' '))
    .replace(/[:：]\s*$/, '').trim();
const labelText = (l) => {
  const c = l.cloneNode(true);
  c.querySelectorAll('input,select,textarea,option,button,script,style,svg').forEach((x) => x.remove());
  return norm(c.textContent);
};
const fromAria = (el) => {
  const ids = el.getAttribute('aria-labelledby');
  if (ids) {
    const t = norm(ids.split(/\s+/).map((id) => textOf(document.getElementById(id))).join(' '));
    if (t) return t;
  }
  return norm(el.getAttribute('aria-label'));
};
const hasControl = (n) => n.matches && (n.matches('input,select,textarea') || n.querySelector('input,select,textarea'));
const nearby = (el) => {
  let n = el;
  for (let d = 0; d < 5 && n && n !== document.body; d++, n = n.parentElement) {
    let s = n.previousElementSibling;
    for (let k = 0; s && k < 2; k++, s = s.previousElementSibling) {
      if (hasControl(s)) break;
      const t = textOf(s);
      if (t && t.length <= 300) return t;
    }
  }
  return '';
};
const leading = (anc, members) => {
  let best = '';
  for (const ch of anc.children) {
    if (members.some((m) => ch.contains(m))) break;
    if (hasControl(ch)) continue;
    const t = textOf(ch);
    if (t && t.length <= 300) best = t;
  }
  return best;
};
const labelFor = (el) => {
  let raw = fromAria(el);
  if (!raw && el.labels && el.labels.length) raw = norm([...el.labels].map(labelText).join(' '));
  if (!raw) { const l = el.closest('label'); if (l) raw = labelText(l); }
  if (!raw) raw = nearby(el);
  if (!raw) raw = norm(el.getAttribute('placeholder'));
  if (!raw) raw = norm(el.getAttribute('title'));
  if (!raw) raw = norm((el.getAttribute('name') || el.id || '').replace(/[_\-\[\]]+/g, ' '));
  return raw;
};
const optLabel = (el) => {
  if (el.labels && el.labels.length) { const t = norm([...el.labels].map(labelText).join(' ')); if (t) return cleanLabel(t); }
  const al = norm(el.getAttribute('aria-label')); if (al) return cleanLabel(al);
  const lb = el.closest('label'); if (lb) { const t = labelText(lb); if (t) return cleanLabel(t); }
  for (let s = el.nextSibling; s; s = s.nextSibling) { const t = norm(s.textContent); if (t) return cleanLabel(t); }
  return cleanLabel(el.value || '');
};
const groupLabel = (members) => {
  const first = members[0];
  const fs = first.closest('fieldset');
  if (fs) { const lg = fs.querySelector('legend'); if (lg) { const t = textOf(lg); if (t) return t; } }
  const rg = first.closest('[role=radiogroup],[role=group]');
  if (rg) { const t = fromAria(rg); if (t) return t; }
  let anc = first.parentElement;
  while (anc && !members.every((m) => anc.contains(m))) anc = anc.parentElement;
  if (anc) { const t = leading(anc, members) || nearby(anc); if (t) return t; }
  return norm((first.getAttribute('name') || '').replace(/[_\-\[\]]+/g, ' '));
};
const same = (a, b) => norm(a).toLowerCase() === norm(b).toLowerCase();
"#;

/// Tag the visible fields of the current form step and describe them.
/// Args: `{step: number, root: string[]}`.
const EXTRACT: &str = r#"
const ARGS = __ARGS__;
const prefix = 'jp' + ARGS.step + '_';
document.querySelectorAll('[data-jp-id]').forEach((e) => {
  if ((e.getAttribute('data-jp-id') || '').startsWith(prefix)) e.removeAttribute('data-jp-id');
});
const rootEl = rootOf(ARGS.root) || document;
const SKIP = new Set(['hidden', 'submit', 'button', 'reset', 'image', 'password', 'range', 'color']);
const typeOf = (el) => {
  const tag = el.tagName.toLowerCase();
  if (tag === 'select') return 'select';
  if (tag === 'textarea') return 'textarea';
  const t = (el.getAttribute('type') || 'text').toLowerCase();
  if (SKIP.has(t)) return null;
  if (t === 'text' || t === 'search') {
    if (el.getAttribute('role') === 'combobox' || el.getAttribute('aria-autocomplete') === 'list') return 'combobox';
    return 'text';
  }
  if (['email', 'tel', 'url', 'number', 'date', 'radio', 'checkbox', 'file'].includes(t)) return t;
  return 'text';
};
const usable = (el, type) => {
  if (el.disabled || !shown(el)) return false;
  if (type === 'file' || type === 'radio' || type === 'checkbox') return true;
  const r = el.getBoundingClientRect();
  if (r.width < 2 || r.height < 2) return false;
  if (el.getAttribute('aria-hidden') === 'true') return false;
  if (el.readOnly && type !== 'combobox') return false;
  return true;
};
const controls = [...rootEl.querySelectorAll('input, select, textarea')]
  .map((el) => ({ el, type: typeOf(el) }))
  .filter((c) => c.type && usable(c.el, c.type));
const key = (el) => (el.form ? [...document.forms].indexOf(el.form) : -1) + '|' + el.type + '|' + el.name;
const byKey = new Map();
for (const c of controls) {
  if ((c.type === 'radio' || c.type === 'checkbox') && c.el.name) {
    const k = key(c.el);
    if (!byKey.has(k)) byKey.set(k, []);
    byKey.get(k).push(c.el);
  }
}
const out = [];
const done = new Set();
let n = 0;
const emit = (f, els) => {
  f.id = prefix + n++;
  f.step = ARGS.step;
  els.forEach((e) => e.setAttribute('data-jp-id', f.id));
  out.push(f);
};
const isRequired = (el, raw) =>
  !!el.required || el.getAttribute('aria-required') === 'true' || /\*|\(required\)/i.test(raw);
for (const { el, type } of controls) {
  if (done.has(el)) continue;
  if (type === 'radio' || (type === 'checkbox' && el.name && (byKey.get(key(el)) || []).length >= 2)) {
    const members = el.name ? byKey.get(key(el)) : [el];
    members.forEach((m) => done.add(m));
    members.forEach((m) => m.setAttribute('data-jp-opt', optLabel(m)));
    const raw = groupLabel(members);
    const checked = members.filter((m) => m.checked).map((m) => m.getAttribute('data-jp-opt'));
    emit({
      label: cleanLabel(raw),
      type: type === 'radio' ? 'radio' : 'checkbox_group',
      required: members.some((m) => isRequired(m, '')) || /\*|\(required\)/i.test(raw),
      options: members.map((m) => m.getAttribute('data-jp-opt')),
      current_value: checked.join('; '),
      name: el.name || '',
    }, members);
    continue;
  }
  let raw = labelFor(el);
  if (type === 'file' && (!raw || (raw.length < 24 && /^(attach|upload|choose|browse|select|drop|add)/i.test(raw)))) {
    // The label of an upload widget is often just its button: use the heading above the widget.
    let a = el.parentElement;
    for (let d = 0; d < 5 && a && a !== document.body; d++, a = a.parentElement) {
      const t = leading(a, [el]) || nearby(a);
      if (t && !/^(attach|upload|dropbox|google drive|enter manually|or)\b/i.test(t)) { raw = t; break; }
    }
  }
  const f = { label: cleanLabel(raw), type, required: isRequired(el, raw), options: [], current_value: '', name: el.name || '' };
  if (type === 'select') {
    f.options = [...el.options]
      .filter((o) => o.value !== '' && !o.disabled)
      .map((o) => norm(o.textContent))
      .filter((t) => t && !/^(select|choose|please select|--)/i.test(t));
    f.current_value = el.selectedIndex >= 0 && el.value !== '' ? norm(el.options[el.selectedIndex].textContent) : '';
  } else if (type === 'checkbox') {
    f.current_value = el.checked ? 'yes' : 'no';
  } else if (type === 'file') {
    f.current_value = [...(el.files || [])].map((x) => x.name).join('; ');
    f.accept = el.getAttribute('accept') || '';
  } else {
    f.current_value = el.value || '';
    if (el.maxLength > 0 && el.maxLength < 100000) f.maxlength = el.maxLength;
  }
  emit(f, [el]);
}
return out;
"#;

/// Set values. Args: `[{id, type, value}]`. Files and comboboxes are skipped (driven over CDP).
/// Returns `[{id, ok, now, error?}]`.
const FILL: &str = r#"
const ITEMS = __ARGS__;
const setValue = (el, v) => {
  const desc = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value');
  if (el.focus) el.focus();
  desc.set.call(el, v);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  if (el.blur) el.blur();
};
const truthy = (v) => /^(yes|true|y|1|on|checked|agree|i agree)$/i.test(norm(v));
const results = [];
for (const it of ITEMS) {
  const els = [...document.querySelectorAll('[data-jp-id="' + it.id + '"]')];
  const r = { id: it.id, ok: false, now: '' };
  try {
    if (!els.length) { r.error = 'field is no longer on the page'; results.push(r); continue; }
    const el = els[0];
    const v = String(it.value == null ? '' : it.value);
    switch (it.type) {
      case 'select': {
        const o = [...el.options].find((o) => same(o.textContent, v) || same(o.value, v));
        if (!o) { r.error = 'option not found'; break; }
        setValue(el, o.value);
        r.now = el.selectedIndex >= 0 ? norm(el.options[el.selectedIndex].textContent) : '';
        r.ok = same(r.now, o.textContent);
        break;
      }
      case 'radio': {
        const m = els.find((m) => same(m.getAttribute('data-jp-opt'), v) || same(m.value, v));
        if (!m) { r.error = 'option not found'; break; }
        if (!m.checked) m.click();
        r.now = m.checked ? m.getAttribute('data-jp-opt') : '';
        r.ok = m.checked;
        break;
      }
      case 'checkbox_group': {
        const wanted = new Set(v.split(';').map((s) => norm(s).toLowerCase()).filter(Boolean));
        for (const m of els) {
          const want = wanted.has(norm(m.getAttribute('data-jp-opt')).toLowerCase());
          if (m.checked !== want) m.click();
        }
        r.now = els.filter((m) => m.checked).map((m) => m.getAttribute('data-jp-opt')).join('; ');
        r.ok = els.every((m) => m.checked === wanted.has(norm(m.getAttribute('data-jp-opt')).toLowerCase()));
        break;
      }
      case 'checkbox': {
        const want = truthy(v);
        if (el.checked !== want) el.click();
        r.now = el.checked ? 'yes' : 'no';
        r.ok = el.checked === want;
        break;
      }
      case 'file':
      case 'combobox':
        r.error = 'skipped';
        break;
      default: {
        setValue(el, v);
        r.now = el.value;
        r.ok = norm(el.value) === norm(v) || (it.type === 'date' && el.value !== '');
        if (!r.ok) r.error = 'the page did not keep the value';
      }
    }
  } catch (e) {
    r.error = String(e && e.message || e);
  }
  results.push(r);
}
return results;
"#;

/// Find a visible, enabled button and tag it `data-jp-btn=<kind>`.
/// Args: `{kind, root: string[], selectors: string[], text: string(regex source)}`.
const FIND_BUTTON: &str = r#"
const ARGS = __ARGS__;
document.querySelectorAll('[data-jp-btn="' + ARGS.kind + '"]').forEach((e) => e.removeAttribute('data-jp-btn'));
const rootEl = rootOf(ARGS.root) || document;
const re = new RegExp('^(?:' + ARGS.text + ')$', 'i');
const label = (e) => norm(e.innerText || e.value || e.getAttribute('aria-label') || '');
const usable = (e) => shown(e) && !e.disabled && e.getAttribute('aria-disabled') !== 'true';
let cands = [];
for (const s of ARGS.selectors) {
  try { cands.push(...rootEl.querySelectorAll(s)); } catch (e) {}
}
cands = cands.filter(usable);
if (!cands.length) {
  cands = [...rootEl.querySelectorAll('button, input[type=submit], input[type=button], a[role=button], a.btn, a.button, [role=button]')]
    .filter((e) => usable(e) && re.test(label(e)));
}
if (!cands.length && rootEl !== document) {
  cands = [...document.querySelectorAll('button, input[type=submit], a[role=button], a.btn, a.button')]
    .filter((e) => usable(e) && re.test(label(e)));
}
const el = cands[0];
if (!el) return { found: false, text: '' };
el.setAttribute('data-jp-btn', ARGS.kind);
return { found: true, text: label(el) };
"#;

/// What the page looks like now. Args: `{root: string[]}`.
const SIGNALS: &str = r#"
const ARGS = __ARGS__;
const rootEl = rootOf(ARGS.root);
const fillable = (e) => !['hidden', 'submit', 'button', 'reset', 'image'].includes((e.getAttribute('type') || '').toLowerCase()) && !e.disabled && shown(e);
const fields = rootEl ? [...rootEl.querySelectorAll('input, select, textarea')].filter(fillable).length : 0;
const frames = [...document.querySelectorAll('iframe')].map((f) => {
  const r = f.getBoundingClientRect();
  return { src: f.src || f.getAttribute('data-src') || '', w: r.width, h: r.height, visible: shown(f), badge: !!f.closest('.grecaptcha-badge') };
});
const errs = new Set();
document.querySelectorAll('input:invalid, select:invalid, textarea:invalid, [aria-invalid="true"]').forEach((e) => {
  if (shown(e) && !e.disabled && e.tagName !== 'FORM') errs.add(cleanLabel(labelFor(e)) + ': missing or invalid');
});
document.querySelectorAll('[role=alert], .error, .errors, .field-error, .error-message, .invalid-feedback, [class*="error-"]').forEach((e) => {
  if (shown(e) && !e.matches('input,select,textarea')) { const t = textOf(e); if (t && t.length <= 200) errs.add(t); }
});
const body = document.body ? (document.body.innerText || '') : '';
return {
  url: location.href,
  title: document.title || '',
  text: body.slice(0, 6000),
  field_count: fields,
  frames,
  errors: [...errs].slice(0, 6),
};
"#;

/// Page rectangle of the form root (for the filled-form screenshot). Args: `{root: string[]}`.
const ROOT_RECT: &str = r#"
const ARGS = __ARGS__;
const el = rootOf(ARGS.root);
if (!el) return null;
const r = el.getBoundingClientRect();
if (r.width < 50 || r.height < 50) return null;
return { x: r.left + window.scrollX, y: r.top + window.scrollY, w: r.width, h: r.height };
"#;

/// Texts of the visible dropdown options. No args.
const OPTIONS: &str = r#"
const seen = new Set();
const out = [];
for (const o of document.querySelectorAll('[role=option]')) {
  if (!shown(o)) continue;
  const t = textOf(o);
  if (t && !seen.has(t) && out.length < 200) { seen.add(t); out.push(t); }
}
return out;
"#;

/// Does the page link to an embedded ATS form? Returns the iframe URL or `null`.
const EMBEDDED_FORM: &str = r#"
const f = [...document.querySelectorAll('iframe')].find((f) =>
  /(greenhouse\.io|lever\.co|ashbyhq\.com)/i.test(f.src || '') && /(job_app|apply|application|embed)/i.test(f.src || ''));
return f ? f.src : null;
"#;

fn compose(body: &str, args: &serde_json::Value) -> String {
    let json = serde_json::to_string(args).unwrap_or_else(|_| "null".into());
    format!("(() => {{\n{LIB}\n{}\n}})()", body.replace("__ARGS__", &json))
}

pub fn extract(step: usize, root: &[&str]) -> String {
    compose(EXTRACT, &serde_json::json!({ "step": step, "root": root }))
}

pub fn fill(items: &serde_json::Value) -> String {
    compose(FILL, items)
}

pub fn find_button(kind: &str, root: &[&str], selectors: &[&str], text: &str) -> String {
    compose(FIND_BUTTON, &serde_json::json!({ "kind": kind, "root": root, "selectors": selectors, "text": text }))
}

pub fn signals(root: &[&str]) -> String {
    compose(SIGNALS, &serde_json::json!({ "root": root }))
}

pub fn root_rect(root: &[&str]) -> String {
    compose(ROOT_RECT, &serde_json::json!({ "root": root }))
}

pub fn options() -> String {
    compose(OPTIONS, &serde_json::Value::Null)
}

pub fn embedded_form() -> String {
    compose(EMBEDDED_FORM, &serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_embed_their_arguments_as_json() {
        let js = extract(2, &["form#a", "main form"]);
        assert!(js.contains(r#"{"root":["form#a","main form"],"step":2}"#) || js.contains(r#""step":2"#));
        assert!(!js.contains("__ARGS__"));
        // Hostile text cannot break out of the JSON literal.
        let js = fill(&serde_json::json!([{"id": "x", "type": "text", "value": "\"); alert(1); (\""}]));
        assert!(js.contains(r#"\"); alert(1); (\""#));
        assert!(find_button("submit", &[], &["button[type=submit]"], "submit").contains("data-jp-btn"));
    }
}
