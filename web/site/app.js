// Raft playground.
//
// The page owns the editor and the results; the VM itself lives in a worker
// (see worker.js) so that a program which never halts can be stopped without
// taking the tab with it.

'use strict';

// Generous enough for any program worth typing into a text box, small enough
// that an accidental endless loop answers in well under a second.
const INSTRUCTION_BUDGET = 5_000_000;

// The backstop for everything a budget cannot bound -- a blocked receive, a
// pathological allocation -- after which the worker is killed and replaced.
const RUN_TIMEOUT_MS = 6_000;

const STORAGE_KEY = 'raft.playground.source';

const WELCOME = `# Raft is postfix: operands first, then the instruction that
# consumes them. Press Ctrl+Enter (Cmd+Enter on a Mac) to run.

0 StoreVar 0        # slot 0: running total
5 StoreVar 1        # slot 1: counter

.loop
LoadVar 1 0 Gt      # counter > 0 ?
JumpIfFalse .done
  LoadVar 0 LoadVar 1 + StoreVar 0
  LoadVar 1 1 - StoreVar 1
Jump .loop

.done
LoadVar 0
io.print CallNative 1
`;

const KEYWORDS = new Set([
  'Pop', 'Dup', 'Swap',
  'StoreVar', 'LoadVar', 'LoadGlobal', 'GetExport',
  'MakeArray', 'ArrayGet', 'ArraySet',
  'MakeString', 'PushString', 'StringConcat',
  'MakeModule', 'ModuleGet', 'ModuleSet',
  'CallNative',
  'Add', 'Sub', 'Mul', 'Div', 'Mod', 'Neg', 'Exp',
  'Eq', 'Ne', 'Lt', 'Le', 'Gt', 'Ge',
  'Not', 'And', 'Or',
  'Jump', 'JumpIfFalse', 'Call', 'Return',
  'SpawnActor', 'SendMessage', 'ReceiveMessage',
  'SpawnSupervisor', 'SetStrategy', 'SuperviseChild', 'RestartChild',
]);

const element = (id) => document.getElementById(id);

const dom = {
  source: element('source'),
  highlight: element('highlight').firstElementChild,
  gutter: element('gutter'),
  caret: element('caret'),
  run: element('run'),
  share: element('share'),
  examples: element('examples'),
  version: element('version'),
  banner: element('banner'),
  output: element('output'),
  stack: element('stack'),
  stackEmpty: element('stack-empty'),
  stackCount: element('stack-count'),
  bytecode: element('bytecode'),
  bytecodeEmpty: element('bytecode-empty'),
  bytecodeCount: element('bytecode-count'),
  status: element('status'),
  limits: element('limits'),
};

let worker = null;
let pending = null;
let nextRunId = 1;
let finishedRuns = 0;
let errorLine = 0;

// ------------------------------------------------------------------ editor

function escapeHtml(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

// One pass per line: Raft's strings and comments both end at the newline, so a
// line is a complete unit and marking one is just wrapping its output.
const TOKEN = new RegExp(
  [
    '(#[^\\n]*|//[^\\n]*)',            // 1 comment
    '("(?:[^"\\\\]|\\\\.)*"?)',        // 2 string
    '(\\.[A-Za-z_][\\w.]*)',           // 3 label
    '\\b(true|false)\\b',              // 4 boolean
    '(-?\\d+\\.\\d+|-?\\d+)',          // 5 number
    '([A-Za-z_][\\w.]*)',              // 6 identifier
    '([+\\-*/%^])',                    // 7 operator
  ].join('|'),
  'g'
);

function highlightLine(line) {
  let html = '';
  let index = 0;
  TOKEN.lastIndex = 0;

  for (let match = TOKEN.exec(line); match; match = TOKEN.exec(line)) {
    html += escapeHtml(line.slice(index, match.index));
    index = match.index + match[0].length;

    const text = escapeHtml(match[0]);
    if (match[1]) html += `<span class="tok-comment">${text}</span>`;
    else if (match[2]) html += `<span class="tok-string">${text}</span>`;
    else if (match[3]) html += `<span class="tok-label">${text}</span>`;
    else if (match[4]) html += `<span class="tok-boolean">${text}</span>`;
    else if (match[5]) html += `<span class="tok-number">${text}</span>`;
    else if (match[6]) {
      const name = match[6];
      const kind = KEYWORDS.has(name)
        ? 'tok-keyword'
        : name.includes('.')
          ? 'tok-native'
          : '';
      html += kind ? `<span class="${kind}">${text}</span>` : text;
    } else html += `<span class="tok-operator">${text}</span>`;
  }

  return html + escapeHtml(line.slice(index));
}

function paint() {
  const lines = dom.source.value.split('\n');

  dom.highlight.innerHTML = lines
    .map((line, index) => {
      const rendered = highlightLine(line) || '&nbsp;';
      return index + 1 === errorLine
        ? `<span class="mark-error">${rendered}</span>`
        : rendered;
    })
    .join('\n');

  dom.gutter.innerHTML = lines
    .map((_, index) =>
      index + 1 === errorLine
        ? `<span class="error-line">${index + 1}</span>`
        : `<span>${index + 1}</span>`
    )
    .join('');

  syncScroll();
}

function syncScroll() {
  dom.highlight.parentElement.scrollTop = dom.source.scrollTop;
  dom.highlight.parentElement.scrollLeft = dom.source.scrollLeft;
  dom.gutter.scrollTop = dom.source.scrollTop;
}

function caretPosition() {
  const upToCaret = dom.source.value.slice(0, dom.source.selectionStart);
  const lines = upToCaret.split('\n');
  return { line: lines.length, column: lines[lines.length - 1].length + 1 };
}

function showCaret() {
  const { line, column } = caretPosition();
  dom.caret.textContent = `line ${line}, col ${column}`;
}

function goToLine(line) {
  const lines = dom.source.value.split('\n');
  if (line < 1 || line > lines.length) return;

  const start = lines.slice(0, line - 1).reduce((total, text) => total + text.length + 1, 0);
  dom.source.focus();
  dom.source.setSelectionRange(start, start + lines[line - 1].length);

  // Put the line a third of the way down rather than at the very top, which
  // is where a bare scrollTop would leave it.
  const lineHeight = dom.source.scrollHeight / Math.max(lines.length, 1);
  dom.source.scrollTop = Math.max(0, (line - 1) * lineHeight - dom.source.clientHeight / 3);
  syncScroll();
  showCaret();
}

function insertText(text) {
  // execCommand keeps the browser's own undo history intact, which setting
  // .value directly would throw away.
  if (document.execCommand && document.execCommand('insertText', false, text)) return;

  const { selectionStart: start, selectionEnd: end, value } = dom.source;
  dom.source.value = value.slice(0, start) + text + value.slice(end);
  dom.source.setSelectionRange(start + text.length, start + text.length);
  dom.source.dispatchEvent(new Event('input'));
}

// ------------------------------------------------------------------- tabs

const tabs = Array.from(document.querySelectorAll('[role="tab"]'));

function selectTab(tab) {
  for (const candidate of tabs) {
    const selected = candidate === tab;
    candidate.setAttribute('aria-selected', String(selected));
    candidate.tabIndex = selected ? 0 : -1;
    element(candidate.getAttribute('aria-controls')).hidden = !selected;
  }
}

function activeTab() {
  return tabs.find((tab) => tab.getAttribute('aria-selected') === 'true');
}

for (const [index, tab] of tabs.entries()) {
  tab.addEventListener('click', () => selectTab(tab));
  tab.addEventListener('keydown', (event) => {
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = tabs[(index + step + tabs.length) % tabs.length];
    selectTab(next);
    next.focus();
  });
}

// ----------------------------------------------------------------- running

function createWorker() {
  worker = new Worker('worker.js');
  worker.onmessage = onWorkerMessage;
  worker.onerror = (event) => {
    setBusy(false);
    showFailure(event.message || 'the VM worker stopped unexpectedly');
  };
  worker.postMessage({ type: 'init' });
}

function setBusy(busy) {
  dom.run.disabled = busy;
  dom.run.textContent = busy ? 'Running…' : 'Run ';
  if (!busy) {
    dom.run.insertAdjacentHTML('beforeend', '<kbd>Ctrl</kbd><kbd>↵</kbd>');
    document.body.dataset.runs = String(++finishedRuns);
  }
  // Anything driving this page -- a test, a screenshot script -- can wait on
  // these rather than guessing from the button.
  document.body.dataset.state = busy ? 'running' : 'idle';
}

function run() {
  if (!worker || pending) return;

  if (activeTab() && activeTab().id === 'tab-reference') selectTab(element('tab-output'));

  const id = nextRunId++;
  setBusy(true);
  setStatus('Running…');

  pending = {
    id,
    started: performance.now(),
    timer: setTimeout(() => onTimeout(id), RUN_TIMEOUT_MS),
  };

  worker.postMessage({
    type: 'run',
    id,
    source: dom.source.value,
    budget: INSTRUCTION_BUDGET,
  });
}

function onTimeout(id) {
  if (!pending || pending.id !== id) return;

  worker.terminate();
  pending = null;
  createWorker();
  setBusy(false);

  showFailure(
    `Stopped after ${(RUN_TIMEOUT_MS / 1000).toFixed(0)}s. The program was still ` +
      'running -- an actor waiting on a message that never came will do this.'
  );
  setStatus('<span class="bad">stopped</span> · the run outlasted its stopwatch');
}

function onWorkerMessage(event) {
  const message = event.data || {};

  if (message.type === 'ready') {
    dom.version.textContent = `v${message.info.version}`;
    dom.limits.textContent =
      `Instruction budget: ${INSTRUCTION_BUDGET.toLocaleString()} · ` +
      `run stopped after ${RUN_TIMEOUT_MS / 1000}s · ` +
      `output kept: ${message.info.maxOutputLines.toLocaleString()} lines · ` +
      `stack shown: ${message.info.maxStackEntries} entries`;
    return;
  }

  if (!pending || pending.id !== message.id) return;

  clearTimeout(pending.timer);
  const elapsed = performance.now() - pending.started;
  pending = null;
  setBusy(false);

  if (message.type === 'result') showReport(message.report, elapsed);
  else if (message.type === 'failed') showFailure(message.error);
}

// ----------------------------------------------------------------- results

function showFailure(text) {
  errorLine = 0;
  paint();
  dom.banner.hidden = false;
  dom.banner.textContent = text;
  setStatus('<span class="bad">error</span>');
}

function setStatus(html) {
  dom.status.innerHTML = html;
}

function showReport(report, elapsed) {
  renderBanner(report);
  renderOutput(report);
  renderStack(report);
  renderBytecode(report);
  renderStatus(report, elapsed);
  paint();
}

function renderBanner(report) {
  errorLine = 0;

  if (report.ok) {
    dom.banner.hidden = true;
    dom.banner.textContent = '';
    return;
  }

  dom.banner.hidden = false;
  dom.banner.textContent = '';

  const stage = report.stage === 'compile' ? 'Compile error' : 'Runtime error';
  dom.banner.append(`${stage}: ${report.error}`);

  if (report.line) {
    errorLine = report.line;
    const where = document.createElement('span');
    where.className = 'where';
    where.textContent = report.column
      ? ` (line ${report.line}, column ${report.column})`
      : ` (line ${report.line})`;
    dom.banner.append(where);

    const jump = document.createElement('button');
    jump.type = 'button';
    jump.textContent = 'Go to line';
    jump.addEventListener('click', () => goToLine(report.line));
    dom.banner.append(jump);
  }
}

function renderOutput(report) {
  const lines = report.output || [];
  dom.output.textContent = lines.join('\n');

  if (lines.length === 0) {
    const note =
      report.stackDepth > 0
        ? 'The program printed nothing. Its result is on the stack — see the Stack tab.'
        : 'The program printed nothing.';
    dom.output.innerHTML = `<span class="muted">${note}</span>`;
  }

  if (report.outputDropped > 0) {
    const note = document.createElement('span');
    note.className = 'muted';
    note.textContent = `\n… ${report.outputDropped.toLocaleString()} more lines not shown.`;
    dom.output.append(note);
  }
}

function renderStack(report) {
  const entries = report.stack || [];
  dom.stack.innerHTML = '';
  dom.stackCount.textContent = report.stackDepth ? `(${report.stackDepth})` : '';
  dom.stackEmpty.hidden = entries.length > 0;

  for (const [index, entry] of entries.entries()) {
    const row = document.createElement('li');

    const depth = document.createElement('span');
    depth.className = 'depth';
    depth.textContent = index === 0 ? 'top' : `-${index}`;

    const kind = document.createElement('span');
    kind.className = 'kind';
    kind.textContent = entry.kind;

    const value = document.createElement('span');
    value.className = 'value';
    value.textContent = entry.text;

    row.append(depth, kind, value);
    dom.stack.append(row);
  }

  const hidden = (report.stackDepth || 0) - entries.length;
  if (hidden > 0) {
    const row = document.createElement('li');
    row.className = 'muted';
    row.textContent = `… ${hidden.toLocaleString()} deeper entries not shown`;
    dom.stack.append(row);
  }
}

function renderBytecode(report) {
  const instructions = report.bytecode || [];
  dom.bytecode.innerHTML = '';
  dom.bytecodeCount.textContent = instructions.length ? `(${instructions.length})` : '';
  dom.bytecodeEmpty.hidden = instructions.length > 0;

  for (const instruction of instructions) {
    const row = document.createElement('tr');

    const index = document.createElement('td');
    index.className = 'index';
    index.textContent = instruction.index;

    const text = document.createElement('td');
    text.textContent = instruction.text;

    const line = document.createElement('td');
    line.className = 'line';
    line.textContent = instruction.line == null ? '' : instruction.line;

    row.append(index, text, line);
    if (instruction.line) {
      row.title = `Go to line ${instruction.line}`;
      row.addEventListener('click', () => goToLine(instruction.line));
    }
    dom.bytecode.append(row);
  }
}

function renderStatus(report, elapsed) {
  const heap = report.heap || { live: 0, slots: 0 };
  const parts = [
    report.ok ? '<span class="ok">ok</span>' : '<span class="bad">error</span>',
    `${(report.instructions || 0).toLocaleString()} instructions`,
    `heap ${heap.live}/${heap.slots} live`,
    `${elapsed.toFixed(1)} ms`,
  ];

  if (!report.ok && /budget/i.test(report.error || '')) {
    parts.push('<span class="warn">budget exhausted — is the program looping?</span>');
  }

  setStatus(parts.join(' · '));
}

// ---------------------------------------------------------------- examples

async function loadExamples() {
  try {
    const response = await fetch('examples/index.json');
    if (!response.ok) return;

    const examples = await response.json();
    for (const example of examples) {
      const option = document.createElement('option');
      option.value = example.file;
      option.textContent = example.title;
      dom.examples.append(option);
    }
  } catch (error) {
    // A missing manifest only costs the dropdown; the editor still works.
    console.warn('examples unavailable:', error);
  }
}

dom.examples.addEventListener('change', async () => {
  const file = dom.examples.value;
  if (!file) return;

  try {
    const response = await fetch(`examples/${file}`);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    setSource(await response.text());
    run();
  } catch (error) {
    showFailure(`Could not load ${file}: ${error.message}`);
  } finally {
    dom.examples.selectedIndex = 0;
  }
});

// ------------------------------------------------------------------ links

function encodeSource(source) {
  const bytes = new TextEncoder().encode(source);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function decodeSource(encoded) {
  const padded = encoded.replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

async function copyLink() {
  const url = new URL(window.location.href);
  url.hash = `c=${encodeSource(dom.source.value)}`;

  try {
    await navigator.clipboard.writeText(url.toString());
    setStatus('<span class="ok">link copied</span> · it carries the program in the URL');
  } catch (error) {
    // Clipboard access can be refused; leaving the link in the address bar
    // still lets it be copied by hand.
    window.history.replaceState(null, '', url.toString());
    setStatus('link put in the address bar · copying was blocked');
  }
}

function setSource(source) {
  dom.source.value = source;
  errorLine = 0;
  paint();
  showCaret();
  save();
}

let saveTimer = null;
function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      localStorage.setItem(STORAGE_KEY, dom.source.value);
    } catch (error) {
      // Private windows and blocked storage are fine; the program is still
      // shareable through the link.
    }
  }, 400);
}

function initialSource() {
  const hash = window.location.hash.replace(/^#/, '');
  if (hash.startsWith('c=')) {
    try {
      return decodeSource(hash.slice(2));
    } catch (error) {
      console.warn('unreadable program in the link:', error);
    }
  }

  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) return stored;
  } catch (error) {
    // No storage, no restore.
  }

  return WELCOME;
}

// ------------------------------------------------------------------- setup

dom.source.addEventListener('input', () => {
  if (errorLine) errorLine = 0;
  paint();
  showCaret();
  save();
});
dom.source.addEventListener('scroll', syncScroll);
dom.source.addEventListener('click', showCaret);
dom.source.addEventListener('keyup', showCaret);

dom.source.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    run();
    return;
  }

  if (event.key === 'Tab' && !event.shiftKey) {
    event.preventDefault();
    insertText('  ');
  }
});

document.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    run();
  }
});

dom.run.addEventListener('click', run);
dom.share.addEventListener('click', copyLink);

window.addEventListener('hashchange', () => {
  const hash = window.location.hash.replace(/^#/, '');
  if (!hash.startsWith('c=')) return;
  try {
    setSource(decodeSource(hash.slice(2)));
    run();
  } catch (error) {
    console.warn('unreadable program in the link:', error);
  }
});

setSource(initialSource());
createWorker();
loadExamples();

// Show the program working rather than an empty panel. Runs are bounded and
// take milliseconds, and a shared link is worth nothing if it does not.
run();
