// The playground's VM host.
//
// The VM runs here rather than on the page's thread for one reason: a program
// typed into the editor can loop forever, and only a worker can be terminated
// while it does. The page keeps a stopwatch on every run and kills this worker
// if it overruns, so the tab stays responsive whatever the program does.
//
// Every run gets a fresh instance. Instantiating is cheap next to compiling,
// it keeps one program's heap out of the next program's way, and a program
// that traps leaves its instance unusable -- so reusing one would poison every
// run after it.

const WASM_URL = new URL('raft.wasm', self.location.href);

const decoder = new TextDecoder();
const encoder = new TextEncoder();

let modulePromise = null;

function loadModule() {
  if (!modulePromise) {
    modulePromise = (async () => {
      const response = await fetch(WASM_URL);
      if (!response.ok) {
        throw new Error(`could not load raft.wasm (HTTP ${response.status})`);
      }
      // Plain compile rather than compileStreaming: a static host that serves
      // .wasm as application/octet-stream would make streaming fail, and the
      // module is small enough that buffering it costs nothing.
      return WebAssembly.compile(await response.arrayBuffer());
    })();
  }
  return modulePromise;
}

function instantiate(module) {
  // The module imports nothing, so there is no import object to supply. A
  // panic leaves its message in a buffer at a fixed address instead: the
  // address is taken now, while the instance still answers calls, and read
  // back afterwards even if the instance has trapped.
  const instance = new WebAssembly.Instance(module);
  return { instance, panicPointer: instance.exports.raft_panic_log() };
}

function readString(memory, pointer, length) {
  return decoder.decode(new Uint8Array(memory.buffer, pointer, length));
}

// Read the report the module just produced. The view is built after the call
// returns because running a program can grow the module's memory, which
// detaches any buffer taken before it.
function readReport(exports, length) {
  return JSON.parse(readString(exports.memory, exports.raft_result_ptr(), length));
}

function writeSource(exports, source) {
  const bytes = encoder.encode(source);
  const pointer = exports.raft_alloc(bytes.length);
  if (bytes.length > 0) {
    new Uint8Array(exports.memory.buffer, pointer, bytes.length).set(bytes);
  }
  return { pointer, length: bytes.length };
}

// The message a panicking instance left behind, or '' if it did not panic.
// The first four bytes are the length; the rest is UTF-8.
function readPanic(memory, pointer) {
  try {
    const length = new DataView(memory.buffer).getUint32(pointer, true);
    return length === 0 ? '' : readString(memory, pointer + 4, length);
  } catch (error) {
    // A trap severe enough to lose the memory leaves nothing to read.
    return '';
  }
}

// A trap ends the instance, so whatever the panic hook managed to record is
// all the explanation there will ever be. The one that a program can actually
// provoke is a blocked receive: the scheduler tries to park a thread the
// browser does not have.
function describeTrap(panic, error) {
  if (/condvar|cannot park|no_threads/i.test(panic)) {
    return {
      error: 'Deadlock: every process is blocked waiting for a message, so none can arrive.',
      detail: panic,
    };
  }
  if (panic) {
    return { error: panic.split('\n').slice(-1)[0] || panic, detail: panic };
  }
  return { error: String(error && error.message ? error.message : error), detail: null };
}

async function evaluate(source, budget) {
  const module = await loadModule();
  const { instance, panicPointer } = instantiate(module);
  const exports = instance.exports;
  const started = performance.now();

  try {
    const { pointer, length } = writeSource(exports, source);
    const reportLength = exports.raft_eval(pointer, length, budget);
    const report = readReport(exports, reportLength);
    report.elapsed = performance.now() - started;
    return report;
  } catch (error) {
    const { error: message, detail } = describeTrap(
      readPanic(exports.memory, panicPointer),
      error
    );
    return {
      ok: false,
      stage: 'run',
      error: message,
      detail,
      output: [],
      outputDropped: 0,
      stack: [],
      stackDepth: 0,
      instructions: 0,
      heap: { live: 0, slots: 0 },
      bytecode: [],
      elapsed: performance.now() - started,
    };
  }
}

async function describeBuild() {
  const module = await loadModule();
  const { instance } = instantiate(module);
  return readReport(instance.exports, instance.exports.raft_info());
}

self.onmessage = async (event) => {
  const message = event.data || {};

  try {
    if (message.type === 'init') {
      self.postMessage({ type: 'ready', info: await describeBuild() });
      return;
    }

    if (message.type === 'run') {
      const report = await evaluate(message.source, message.budget);
      self.postMessage({ type: 'result', id: message.id, report });
      return;
    }
  } catch (error) {
    self.postMessage({
      type: 'failed',
      id: message.id,
      error: String(error && error.message ? error.message : error),
    });
  }
};
