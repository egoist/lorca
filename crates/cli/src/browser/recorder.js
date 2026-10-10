// Lorca's recorder for a browser profile, run in the profile's Playwright MCP process through its
// run-code tool: `page` is the server's current tab, `command` is "start" or "stop". It keeps its
// state on the browser context, so it outlives the call that installs it.
//
// The page side listens to the user's own input (trusted events only) and hands each step to a
// binding; this side checks the step, turns the element into Playwright locators, and takes a
// screenshot once the step has settled. A password or a field marked secret is never read.
async (page, command, options) => {
  const context = page.context();
  const MAX_STEPS = 100;
  const MAX_TEXT = 2000;
  const SETTLE_MS = 1200;
  const SECRET_FIELDS = [
    'input[type=password]', '[autocomplete~="current-password" i]', '[autocomplete~="new-password" i]',
    '[autocomplete~="one-time-code" i]', '[autocomplete^="cc-" i]', 'input[name*="pass" i]', 'input[id*="pass" i]',
    'input[name*="secret" i]', 'input[name*="token" i]', 'input[name*="otp" i]',
  ].join(', ');

  // The page side: one listener set per document, alive while the recording is.
  const pageMain = (binding) => {
    const KEY = Symbol.for('lorca.recorder');
    if (window[KEY]) { window[KEY](); return; }
    const send = window[binding];
    if (typeof send !== 'function') return;
    try { delete window[binding]; } catch (e) {}
    const SECRET_AUTOCOMPLETE = /\b(current-password|new-password|one-time-code|cc-number|cc-csc|cc-exp|cc-exp-month|cc-exp-year)\b/i;
    const SECRET_WORDS = /pass(word|code|phrase)?|secret|token|api[-_ ]?key|\botp\b|one[-_ ]?time|verification code|\b2fa\b|\bmfa\b|\bpin\b|\bcvv\b|\bcvc\b|\bcsc\b|security code|\bssn\b/i;
    const TEXT_INPUT = 'input:not([type]), input[type=text], input[type=email], input[type=search], input[type=tel], input[type=url], input[type=number], input[type=password], input[type=date], input[type=time], input[type=datetime-local], input[type=month], input[type=week], input[type=range], input[type=color], textarea';
    const CONTROL = 'input, select, textarea';
    const ACTIONABLE = 'button, a[href], input, select, textarea, summary, label, [contenteditable=""], [contenteditable="true"], [role=button], [role=link], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], [role=tab], [role=option], [role=checkbox], [role=radio], [role=switch], [role=combobox], [role=treeitem], [role=gridcell], [role=textbox], [role=searchbox], [role=slider], [role=spinbutton]';
    const TEST_IDS = ['data-testid', 'data-test-id', 'data-test', 'data-qa', 'data-cy'];
    const wasPassword = new WeakSet();
    let on = false;
    let pending = null;
    let lastDown = null;
    const handlers = [];

    const textOf = (node) => (node ? (node.innerText !== undefined ? node.innerText : node.textContent) || '' : '').replace(/\s+/g, ' ').trim();
    const fit = (text, max) => (text && text.length <= max ? text : '');
    const parentOf = (node) => node.parentElement || (node.getRootNode && node.getRootNode() instanceof ShadowRoot ? node.getRootNode().host : null);
    const deepTarget = (event) => {
      const path = event.composedPath ? event.composedPath() : [];
      return path[0] && path[0].nodeType === 1 ? path[0] : event.target;
    };
    const actionable = (element) => {
      for (let node = element, depth = 0; node && node.nodeType === 1 && depth < 12; node = parentOf(node), depth++) {
        if (node.matches(ACTIONABLE)) return node;
      }
      return element;
    };
    const typeOf = (element) => (element.getAttribute('type') || '').toLowerCase();
    const isTyping = (element) => element.matches(TEXT_INPUT) || element.isContentEditable;

    const roleOf = (element) => {
      const explicit = (element.getAttribute('role') || '').trim().split(/\s+/)[0];
      if (explicit) return explicit;
      const tag = element.tagName.toLowerCase();
      const type = typeOf(element);
      if (tag === 'a' && element.hasAttribute('href')) return 'link';
      if (tag === 'button') return 'button';
      if (tag === 'select') return element.multiple || element.size > 1 ? 'listbox' : 'combobox';
      if (tag === 'textarea') return 'textbox';
      if (tag === 'input') {
        if (['button', 'submit', 'reset', 'image'].includes(type)) return 'button';
        if (type === 'checkbox') return 'checkbox';
        if (type === 'radio') return 'radio';
        if (type === 'range') return 'slider';
        if (type === 'number') return 'spinbutton';
        if (type === 'search') return element.hasAttribute('list') ? 'combobox' : 'searchbox';
        if (['', 'text', 'email', 'tel', 'url'].includes(type)) return element.hasAttribute('list') ? 'combobox' : 'textbox';
        return '';
      }
      if (/^h[1-6]$/.test(tag)) return 'heading';
      return '';
    };
    const labelOf = (element) => {
      const ids = element.getAttribute('aria-labelledby');
      if (ids) {
        const text = ids.split(/\s+/).map((id) => textOf(element.ownerDocument.getElementById(id))).join(' ').trim();
        if (text) return text;
      }
      const aria = (element.getAttribute('aria-label') || '').trim();
      if (aria) return aria;
      if (element.labels && element.labels.length) return textOf(element.labels[0]);
      return '';
    };
    const nameOf = (element, role) => {
      const label = labelOf(element);
      if (label) return label;
      const tag = element.tagName.toLowerCase();
      if (tag === 'input') {
        const type = typeOf(element);
        if (['button', 'submit', 'reset'].includes(type)) return (element.value || '').trim();
        if (type === 'image') return (element.getAttribute('alt') || '').trim();
        return (element.getAttribute('title') || '').trim();
      }
      if (['button', 'link', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'checkbox', 'radio', 'switch', 'treeitem', 'heading', 'gridcell'].includes(role)) {
        const text = textOf(element) || Array.from(element.querySelectorAll('img[alt]')).map((image) => image.alt).join(' ').trim();
        if (text) return text;
      }
      return (element.getAttribute('title') || '').trim();
    };
    const stableId = (id) => !!id && /^[A-Za-z][\w-]{0,63}$/.test(id) && !/\d{3,}/.test(id) && !/^(ember|react|radix|headlessui|mui|rc|r)[-_]?\d/i.test(id);
    const unique = (selector, element) => {
      try { const found = element.ownerDocument.querySelectorAll(selector); return found.length === 1 && found[0] === element; } catch (e) { return false; }
    };
    const pathOf = (element) => {
      const parts = [];
      for (let node = element, depth = 0; node && node.nodeType === 1 && depth < 6; node = node.parentElement, depth++) {
        if (node !== element && stableId(node.id)) { parts.unshift('#' + CSS.escape(node.id)); break; }
        const tag = node.tagName.toLowerCase();
        if (tag === 'body' || tag === 'html') { parts.unshift(tag); break; }
        let index = 1;
        for (let sibling = node.previousElementSibling; sibling; sibling = sibling.previousElementSibling) if (sibling.tagName === node.tagName) index++;
        parts.unshift(tag + ':nth-of-type(' + index + ')');
      }
      return parts.join(' > ');
    };
    // What identifies the element, from the most to the least robust; the recorder's other side
    // makes Playwright locators of it.
    const describe = (element) => {
      const role = roleOf(element);
      const info = { tag: element.tagName.toLowerCase(), role, name: fit(nameOf(element, role), 100), css: [] };
      if (element.matches(CONTROL) || element.isContentEditable) {
        info.label = fit(labelOf(element), 100);
        info.placeholder = fit((element.getAttribute('placeholder') || '').trim(), 100);
      } else {
        info.text = fit(textOf(element), 80);
      }
      for (const attribute of TEST_IDS) {
        const value = element.getAttribute(attribute);
        if (value && value.length <= 80) { info.testid = [attribute, value]; break; }
      }
      if (stableId(element.id) && unique('#' + CSS.escape(element.id), element)) info.css.push('#' + CSS.escape(element.id));
      const name = element.getAttribute('name');
      if (name && name.length <= 80 && element.matches(CONTROL + ', button')) {
        const selector = element.tagName.toLowerCase() + '[name="' + name.replace(/["\\]/g, '\\$&') + '"]';
        if (unique(selector, element)) info.css.push(selector);
      }
      // Where it sits on the page only for an element with nothing else to know it by: one
      // that has would be a guess once the page changes.
      const known = info.name || info.label || info.placeholder || info.text || info.testid || info.css.length;
      const path = pathOf(element);
      if (!known && unique(path, element)) info.css.push(path);
      return info;
    };
    const isSecret = (element) => {
      if (!element || element.nodeType !== 1) return false;
      if (wasPassword.has(element) || typeOf(element) === 'password') return true;
      if (SECRET_AUTOCOMPLETE.test(element.getAttribute('autocomplete') || '')) return true;
      const words = [element.getAttribute('name'), element.id, element.getAttribute('aria-label'), element.getAttribute('placeholder'), labelOf(element)].filter(Boolean).join(' ');
      return SECRET_WORDS.test(words);
    };
    const valueOf = (element) => (element.isContentEditable ? textOf(element) : String(element.value == null ? '' : element.value)).slice(0, 2000);

    const off = () => {
      on = false;
      pending = null;
      for (const [type, handler] of handlers.splice(0)) window.removeEventListener(type, handler, true);
    };
    const post = (step) => {
      if (!on) return;
      step.url = location.href;
      step.title = document.title;
      Promise.resolve(send(step)).then((answer) => { if (answer === false) off(); }, () => {});
    };
    const flush = () => {
      if (!pending) return;
      const { element, info } = pending;
      pending = null;
      if (isSecret(element)) post({ kind: 'fill', target: info, secret: true });
      else post({ kind: 'fill', target: info, value: valueOf(element) });
    };
    const listen = (type, handler) => {
      window.addEventListener(type, handler, true);
      handlers.push([type, handler]);
    };
    const attach = () => {
      listen('pointerdown', (event) => {
        if (!event.isTrusted) return;
        const element = actionable(deepTarget(event));
        if (pending && pending.element !== element) flush();
        lastDown = { element, info: describe(element), at: Date.now() };
      });
      listen('click', (event) => {
        if (!event.isTrusted) return;
        const element = actionable(deepTarget(event));
        const tag = element.tagName.toLowerCase();
        const type = typeOf(element);
        // A click that only puts the caret in a field: what is typed there is the step.
        if (tag === 'select' || isTyping(element)) return;
        if (tag === 'input' && type === 'file') return;
        if (tag === 'label' && element.control && element.control.matches('input[type=checkbox], input[type=radio]')) return;
        const info = lastDown && lastDown.element === element && Date.now() - lastDown.at < 5000 ? lastDown.info : describe(element);
        if (pending) flush();
        if (tag === 'input' && (type === 'checkbox' || type === 'radio')) {
          post({ kind: element.checked ? 'check' : 'uncheck', target: info });
          return;
        }
        post({ kind: 'click', target: info });
      });
      listen('input', (event) => {
        if (!event.isTrusted) return;
        const element = deepTarget(event);
        if (element.nodeType !== 1 || !isTyping(element)) return;
        if (typeOf(element) === 'password') wasPassword.add(element);
        if (!pending || pending.element !== element) {
          flush();
          pending = { element, info: describe(element) };
        }
      });
      listen('change', (event) => {
        if (!event.isTrusted) return;
        const element = deepTarget(event);
        if (element.nodeType !== 1) return;
        if (element.tagName.toLowerCase() === 'select') {
          if (pending) flush();
          const options = Array.from(element.selectedOptions);
          post({ kind: 'select', target: describe(element), values: options.map((option) => option.value), labels: options.map((option) => option.label) });
        } else if (element.matches('input[type=file]')) {
          post({ kind: 'upload', target: describe(element), files: Array.from(element.files || []).map((file) => file.name) });
        } else if (pending && pending.element === element) {
          flush();
        }
      });
      listen('focusout', (event) => {
        if (pending && pending.element === deepTarget(event)) flush();
      });
      listen('keydown', (event) => {
        if (!event.isTrusted) return;
        const element = deepTarget(event);
        if (element.nodeType === 1 && typeOf(element) === 'password') wasPassword.add(element);
        if (event.key === 'Enter' && !event.shiftKey && !(element.nodeType === 1 && element.tagName.toLowerCase() === 'textarea')) {
          flush();
          post({ kind: 'press', key: 'Enter', target: element.nodeType === 1 ? describe(actionable(element)) : null });
        } else if (event.key === 'Escape') {
          flush();
          post({ kind: 'press', key: 'Escape' });
        }
      });
      listen('pagehide', () => flush());
    };
    const activate = () => {
      if (on) return;
      Promise.resolve(send({ kind: 'hello' })).then((answer) => {
        if (answer === true && !on) { on = true; attach(); }
      }, () => {});
    };
    Object.defineProperty(window, KEY, { value: activate });
    activate();
  };

  // Playwright's own quoting for a locator's text, which its locator parser reads back.
  const quote = (text) => {
    const json = JSON.stringify(String(text));
    return "'" + json.slice(1, -1).replace(/\\"/g, '"').replace(/'/g, "\\'") + "'";
  };
  const text = (value, max) => (typeof value === 'string' ? value.slice(0, max) : '');
  const titleOf = (shown) => Promise.race([shown.title(), new Promise((resolve) => setTimeout(() => resolve(''), 1000))]).catch(() => '');
  const words = (value, max) => (typeof value === 'string' && value.length <= max ? value : '');

  // Locators for what the page described, the ones Playwright finds exactly once first. A
  // frame's locators enter it from the page.
  const locators = async (frame, info) => {
    if (!info || typeof info !== 'object') return [];
    const role = words(info.role, 40);
    const name = words(info.name, 100);
    const label = words(info.label, 100);
    const placeholder = words(info.placeholder, 100);
    const shown = words(info.text, 80);
    const testid = Array.isArray(info.testid) ? [words(info.testid[0], 20), words(info.testid[1], 80)] : null;
    const css = Array.isArray(info.css) ? info.css.slice(0, 3).map((selector) => words(selector, 300)).filter(Boolean) : [];
    const options = [];
    if (testid && testid[0] === 'data-testid' && testid[1]) options.push([`getByTestId(${quote(testid[1])})`, (root) => root.getByTestId(testid[1])]);
    else if (testid && /^data-[\w-]+$/.test(testid[0]) && testid[1]) {
      const selector = `[${testid[0]}="${testid[1].replace(/["\\]/g, '\\$&')}"]`;
      options.push([`locator(${quote(selector)})`, (root) => root.locator(selector)]);
    }
    if (role && name) options.push([`getByRole(${quote(role)}, { name: ${quote(name)}, exact: true })`, (root) => root.getByRole(role, { name, exact: true })]);
    if (label) options.push([`getByLabel(${quote(label)}, { exact: true })`, (root) => root.getByLabel(label, { exact: true })]);
    if (placeholder) options.push([`getByPlaceholder(${quote(placeholder)}, { exact: true })`, (root) => root.getByPlaceholder(placeholder, { exact: true })]);
    for (const selector of css.slice(0, -1)) options.push([`locator(${quote(selector)})`, (root) => root.locator(selector)]);
    if (shown) options.push([`getByText(${quote(shown)}, { exact: true })`, (root) => root.getByText(shown, { exact: true })]);
    if (css.length) options.push([`locator(${quote(css[css.length - 1])})`, (root) => root.locator(css[css.length - 1])]);
    let prefix = '';
    try {
      for (let child = frame; child && child.parentFrame(); child = child.parentFrame()) {
        const element = await child.frameElement();
        const selector = await element.evaluate((node) => {
          const attribute = (name) => node.getAttribute(name) && `iframe[${name}="${node.getAttribute(name).replace(/["\\]/g, '\\$&')}"]`;
          return attribute('name') || attribute('title') || (node.id && `#${CSS.escape(node.id)}`) || attribute('src') || 'iframe';
        });
        prefix = `locator(${quote(selector)}).contentFrame().` + prefix;
      }
    } catch (error) {}
    const counts = await Promise.all(options.map(([, make]) => Promise.race([
      Promise.resolve().then(() => make(frame).count()).catch(() => -1),
      new Promise((resolve) => setTimeout(() => resolve(-1), 1500)),
    ])));
    const found = [];
    const unknown = [];
    options.forEach(([code], index) => {
      if (counts[index] === 1) found.push(prefix + code);
      // Gone already (a menu that closed on the click): kept, after the ones that matched.
      else if (counts[index] <= 0) unknown.push(prefix + code);
    });
    return [...new Set([...found, ...unknown])].slice(0, 6);
  };
  const element = (info) => {
    if (!info || typeof info !== 'object') return '';
    const role = words(info.role, 40);
    const name = text(info.name, 100) || text(info.label, 100) || text(info.placeholder, 100) || text(info.text, 80);
    return [name && `“${name}”`, role || words(info.tag, 20)].filter(Boolean).join(' ');
  };

  const shoot = async (recording) => {
    const pending = recording.shot;
    if (!pending) return;
    recording.shot = null;
    clearTimeout(pending.timer);
    const shown = recording.opened && !recording.opened.isClosed() ? recording.opened : pending.page;
    recording.opened = null;
    if (shown.isClosed()) return;
    const file = `step-${pending.index}.jpg`;
    try {
      await shown.screenshot({ path: recording.dir + '/' + file, type: 'jpeg', quality: 60, scale: 'css', timeout: 5000, mask: [shown.locator(SECRET_FIELDS)] });
      pending.step.shot = file;
    } catch (error) {}
    try { pending.step.after = { url: text(shown.url(), 2000), title: text(await titleOf(shown), 200) }; } catch (error) {}
  };
  const later = (recording, step, shown) => {
    const index = recording.steps.length;
    const pending = { step, page: shown, index };
    pending.timer = setTimeout(() => { state.queue = state.queue.then(() => shoot(recording)); }, SETTLE_MS);
    recording.shot = pending;
  };
  const add = async (recording, step, shown, where) => {
    if (recording.steps.length >= MAX_STEPS) { recording.truncated = true; return; }
    try { step.page = where || { url: text(shown.url(), 2000), title: text(await titleOf(shown), 200) }; } catch (error) {}
    recording.steps.push(step);
    recording.at = Date.now();
    later(recording, step, shown);
  };

  const record = async (source, raw) => {
    const recording = state.active;
    if (!recording || !raw || typeof raw !== 'object') return;
    const kind = raw.kind;
    if (!['click', 'fill', 'select', 'check', 'uncheck', 'press', 'upload'].includes(kind)) return;
    // The element first, while it is still on the page; then how the step before ended.
    const targets = await locators(source.frame, raw.target);
    await shoot(recording);
    const step = { action: kind, element: element(raw.target) };
    if (targets.length) step.targets = targets;
    else if (kind !== 'press') return;
    if (kind === 'fill') {
      if (raw.secret === true || typeof raw.value !== 'string') step.secret = true;
      else step.value = raw.value.slice(0, MAX_TEXT);
    } else if (kind === 'select') {
      step.values = (Array.isArray(raw.values) ? raw.values : []).slice(0, 20).map((value) => text(value, 200));
      step.labels = (Array.isArray(raw.labels) ? raw.labels : []).slice(0, 20).map((value) => text(value, 200));
    } else if (kind === 'press') {
      if (!['Enter', 'Escape'].includes(raw.key)) return;
      step.key = raw.key;
      if (!step.targets) delete step.element;
    } else if (kind === 'upload') {
      step.files = (Array.isArray(raw.files) ? raw.files : []).slice(0, 20).map((value) => text(value, 200));
      // The button that opened the file chooser is what the user clicked.
      const last = recording.steps[recording.steps.length - 1];
      if (last && last.action === 'click' && Date.now() - recording.at < 120000) {
        last.action = 'upload';
        last.files = step.files;
        return;
      }
    }
    // Where it happened: the page says, from the main frame, before the step led elsewhere.
    const main = source.frame === source.page.mainFrame() && typeof raw.url === 'string' && /^https?:/i.test(raw.url);
    await add(recording, step, source.page, main ? { url: text(raw.url, 2000), title: text(raw.title, 200) } : null);
  };

  // A page the user went to themselves (the address bar, a bookmark, Back): a step of its own.
  // One a step led to shows in that step's screenshot and address.
  const navigated = (shown, url) => {
    const recording = state.active;
    if (!recording || !/^https?:/i.test(url)) return;
    if (Date.now() - recording.heard < 4000) return;
    state.queue = state.queue.then(async () => {
      await shoot(recording);
      await add(recording, { action: 'goto', url: text(url, 2000) }, shown);
    }).catch(() => {});
  };
  const watch = (shown) => {
    if (state.pages.has(shown)) return;
    state.pages.add(shown);
    shown.on('framenavigated', (frame) => { if (frame === shown.mainFrame()) navigated(shown, frame.url()); });
  };

  let state = context.__lorcaRecorder;
  if (!state) {
    state = { binding: options.binding, installed: false, active: null, queue: Promise.resolve(), pages: new WeakSet() };
    context.__lorcaRecorder = state;
  }

  if (command === 'start') {
    if (state.active) return { lorca: 1, started: false };
    if (!state.installed) {
      await context.exposeBinding(state.binding, (source, raw) => {
        if (raw && raw.kind === 'hello') return !!state.active;
        if (!state.active) return false;
        // When the user last did something: a page that opens soon after is where it led.
        state.active.heard = Date.now();
        state.queue = state.queue.then(() => record(source, raw)).catch(() => {});
        return true;
      });
      await context.addInitScript(`(${pageMain})(${JSON.stringify(state.binding)})`);
      context.on('page', (opened) => {
        watch(opened);
        if (state.active) state.active.opened = opened;
      });
      state.installed = true;
    }
    state.active = { dir: options.dir, steps: [], shot: null, opened: null, at: 0, heard: 0, truncated: false };
    for (const shown of context.pages()) {
      watch(shown);
      for (const frame of shown.frames()) await frame.evaluate(`(${pageMain})(${JSON.stringify(state.binding)})`).catch(() => {});
    }
    if (/^https?:/i.test(page.url())) await add(state.active, { action: 'goto', url: text(page.url(), 2000) }, page);
    return { lorca: 1, started: true };
  }

  if (command === 'stop') {
    const recording = state.active;
    if (!recording) return { lorca: 1, steps: null };
    await state.queue;
    await shoot(recording);
    state.active = null;
    return { lorca: 1, steps: recording.steps, truncated: recording.truncated };
  }
  return { lorca: 1 };
}
