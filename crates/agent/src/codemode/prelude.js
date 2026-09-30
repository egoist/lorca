// Evaluated inside the QuickJS VM before a codemode script runs, after pi's codemode prelude.
//
// The VM is a runtime of its own with no I/O, so nothing here guards a realm boundary. The
// prelude keeps the host bridge in a closure the script cannot reach, and builds `tools`,
// `ALL_TOOLS`, the output helpers (`text`, `image`, `exit`, `console`), `store`/`load`, and
// the host globals on top of it. Arguments and results cross as JSON text. The built-ins it
// relies on are captured before the script runs, and its own state lives in objects with no
// prototype, so a script that patches `Map.prototype` or `Promise` cannot hide a pending call.
// The host checks what crosses the bridge again; nothing here is its only guard.
//
// `bridge(kind, id, text, extra)` takes primitives only:
// - ("call", id, name, argsJson?) and ("global", id, name, argsJson?) ask the host to run a tool
//   or a global; the host answers through `settle`.
// - ("text", 0, text) and ("image", 0, base64, mimeType) append output.
// - ("done", 0, writesJson, valueJson?) and ("failed", 0, errorJson) end the script.
//
// Evaluates to `(bridge, toolsJson, globalsJson, storeJson, limitsJson) => { settle, run, stalled }`.
// `stalled()` says where the script stands once its jobs ran: "waiting" on a host call,
// "finished", or "stalled", which fails a script that waits on nothing: with no timers or I/O
// in the VM, nothing can ever resume it.
(function (bridge, toolsJson, globalsJson, storeJson, limitsJson) {
	"use strict";
	const stringify = JSON.stringify;
	const parse = JSON.parse;
	const PromiseCtor = Promise;
	const promiseThen = Promise.prototype.then;
	const apply = Reflect.apply;
	const objectCreate = Object.create;
	const objectKeys = Object.keys;
	const defineProperty = Object.defineProperty;
	const freeze = Object.freeze;
	const toWellFormed = String.prototype.toWellFormed;
	const replaceString = String.prototype.replace;
	const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g;
	const ErrorCtor = Error;
	const TypeErrorCtor = TypeError;
	const RangeErrorCtor = RangeError;
	const limits = parse(limitsJson);
	// Call id -> { resolve, reject }, and how many there are.
	const pending = objectCreate(null);
	let pendingCount = 0;
	let nextId = 1;
	let finished = false;
	// Thrown by exit() to unwind the script after it already reported success.
	const EXIT = freeze({});

	// A string with every lone surrogate replaced, which is what can cross to the host.
	function wellFormed(text) {
		if (typeof toWellFormed === "function") return apply(toWellFormed, text, []);
		return apply(replaceString, text, [LONE_SURROGATE, "�"]);
	}

	function jsonReplacer(_key, value) {
		return typeof value === "string" ? wellFormed(value) : value;
	}

	function toJson(value) {
		return stringify(value, jsonReplacer);
	}

	// The bridge takes no explicit `undefined`: an absent value is a missing argument.
	function send(kind, id, text, extra) {
		if (extra === undefined) bridge(kind, id, wellFormed(text));
		else bridge(kind, id, wellFormed(text), wellFormed(extra));
	}

	function done(ok, payload, writes) {
		if (finished) return;
		finished = true;
		if (ok) send("done", 0, writes, payload);
		else send("failed", 0, payload);
	}

	function serialize(value) {
		return value === undefined ? undefined : toJson(value);
	}

	// QuickJS stacks list frames only. Prefix "Name: message" like V8 so the text reads the
	// same as a Node error, and drop this prelude's frames.
	function errorText(error) {
		const head = error.message ? error.name + ": " + error.message : String(error.name);
		const frames =
			typeof error.stack === "string"
				? error.stack.split("\n").filter((line) => line.trim() && !line.includes("codemode-prelude.js"))
				: [];
		return [head, ...frames].join("\n");
	}

	function format(value) {
		if (typeof value === "string") return value;
		if (value instanceof ErrorCtor) return errorText(value);
		try {
			const json = toJson(value);
			return json === undefined ? String(value) : json;
		} catch {
			return String(value);
		}
	}

	function describeError(error) {
		if (error === EXIT) return stringify({ message: "exit" });
		// QuickJS throws null when it runs out of memory while making the error itself.
		if (error === null) {
			return stringify({ name: "InternalError", message: "out of memory (or the script threw null)" });
		}
		try {
			if (error instanceof ErrorCtor) {
				return toJson({ name: String(error.name), message: String(error.message), stack: errorText(error) });
			}
			return toJson({ message: format(error) });
		} catch {
			return stringify({ message: "The script threw something that cannot be shown" });
		}
	}

	function caller(kind, name, spread) {
		return (...args) =>
			new PromiseCtor((resolve, reject) => {
				if (finished) {
					reject(new ErrorCtor("The script has already finished"));
					return;
				}
				let json;
				try {
					json = serialize(spread ? args : args[0]);
				} catch (error) {
					reject(error);
					return;
				}
				const id = nextId++;
				pending[id] = { resolve, reject };
				pendingCount++;
				send(kind, id, name, json);
			});
	}

	// Tools known when the script starts, by identifier and by name. A name the script finds
	// later (a searchTools() match) is still callable: the proxy asks the host for it.
	const known = objectCreate(null);
	const allTools = [];
	const toolList = parse(toolsJson);
	for (let i = 0; i < toolList.length; i++) {
		const { name, jsName, description } = toolList[i];
		const fn = caller("call", name);
		// The first tool wins when two names normalize to the same identifier.
		if (!(jsName in known)) {
			known[jsName] = fn;
			allTools[allTools.length] = freeze({ name: jsName, description });
		}
		if (!(name in known)) known[name] = fn;
	}
	freeze(known);
	freeze(allTools);
	const tools = new Proxy(known, {
		get(target, property) {
			if (typeof property !== "string") return undefined;
			if (property in target) return target[property];
			// Awaiting `tools` must not look like a call.
			if (property === "then") return undefined;
			return caller("call", property);
		},
	});

	const namespaces = objectCreate(null);
	const globalList = parse(globalsJson);
	for (let i = 0; i < globalList.length; i++) {
		const { name, spread } = globalList[i];
		const fn = caller("global", name, spread);
		const dot = name.indexOf(".");
		if (dot === -1) {
			defineProperty(globalThis, name, { value: fn, enumerable: true });
			continue;
		}
		const namespace = name.slice(0, dot);
		if (!(namespace in namespaces)) namespaces[namespace] = objectCreate(null);
		namespaces[namespace][name.slice(dot + 1)] = fn;
	}
	const namespaceNames = objectKeys(namespaces);
	for (let i = 0; i < namespaceNames.length; i++) {
		defineProperty(globalThis, namespaceNames[i], { value: freeze(namespaces[namespaceNames[i]]), enumerable: true });
	}

	// key -> JSON text. Sizes count key and JSON characters; the host checks them again.
	const stored = objectCreate(null);
	const initial = parse(storeJson);
	const initialKeys = objectKeys(initial);
	let storedChars = 0;
	for (let i = 0; i < initialKeys.length; i++) {
		const key = initialKeys[i];
		stored[key] = initial[key];
		storedChars += key.length + initial[key].length;
	}
	// key -> JSON text, or null for a deletion.
	const writes = objectCreate(null);

	function checkKey(name, key) {
		if (typeof key !== "string") throw new TypeErrorCtor(name + "() key must be a string");
	}

	function store(key, value) {
		checkKey("store", key);
		key = wellFormed(key);
		const previous = key in stored ? key.length + stored[key].length : 0;
		if (value === undefined) {
			delete stored[key];
			storedChars -= previous;
			writes[key] = null;
			return;
		}
		let json;
		try {
			json = toJson(value);
		} catch (error) {
			throw new TypeErrorCtor("store(" + stringify(key) + ") value is not JSON-serializable: " + format(error));
		}
		if (json === undefined) {
			throw new TypeErrorCtor("store(" + stringify(key) + ") value is not JSON-serializable");
		}
		if (json.length > limits.storeValueChars) {
			throw new RangeErrorCtor("store(" + stringify(key) + ") value exceeds " + limits.storeValueChars + " characters of JSON");
		}
		const next = storedChars - previous + key.length + json.length;
		if (next > limits.storeTotalChars) {
			throw new RangeErrorCtor("store is full: stored values would exceed " + limits.storeTotalChars + " characters of JSON");
		}
		stored[key] = json;
		storedChars = next;
		writes[key] = json;
	}

	function load(key) {
		checkKey("load", key);
		key = wellFormed(key);
		return key in stored ? parse(stored[key]) : undefined;
	}

	// `[[key, json], [key], …]` for the host, built from strings so nothing the script patched
	// takes part.
	function serializeWrites() {
		const keys = objectKeys(writes);
		let out = "[";
		for (let i = 0; i < keys.length; i++) {
			const json = writes[keys[i]];
			out += (i === 0 ? "" : ",") + "[" + stringify(keys[i]) + (json === null ? "" : "," + stringify(json)) + "]";
		}
		return out + "]";
	}

	// Primitives become their string form, everything else JSON.
	function outputText(value) {
		if (value === undefined || value === null || (typeof value !== "object" && typeof value !== "function")) {
			return String(value);
		}
		const json = toJson(value);
		return json === undefined ? String(value) : json;
	}

	function text(value) {
		let rendered;
		try {
			rendered = outputText(value);
		} catch (error) {
			throw new TypeErrorCtor(error instanceof ErrorCtor ? error.message : String(error));
		}
		if (!finished) send("text", 0, rendered);
	}

	const IMAGE_EXPECTS = "image expects a non-empty image URL string, an object with image_url, or an MCP image block";

	function imageUrl(value) {
		if (typeof value === "string") return value;
		if (typeof value !== "object" || value === null || Array.isArray(value)) throw new TypeErrorCtor(IMAGE_EXPECTS);
		if (value.image_url !== undefined) {
			if (typeof value.image_url !== "string") throw new TypeErrorCtor(IMAGE_EXPECTS);
			return value.image_url;
		}
		if (typeof value.type !== "string") throw new TypeErrorCtor(IMAGE_EXPECTS);
		if (value.type !== "image") throw new TypeErrorCtor('image only accepts MCP image blocks, got "' + value.type + '"');
		if (typeof value.data !== "string" || value.data === "") throw new TypeErrorCtor("image expected MCP image data");
		if (value.data.toLowerCase().startsWith("data:")) return value.data;
		const mimeType = typeof value.mimeType === "string" && value.mimeType ? value.mimeType : "application/octet-stream";
		return "data:" + mimeType + ";base64," + value.data;
	}

	function image(value) {
		const url = String(imageUrl(value));
		if (url === "") throw new TypeErrorCtor(IMAGE_EXPECTS);
		const colon = url.indexOf(":");
		const scheme = colon === -1 ? "" : url.slice(0, colon).toLowerCase();
		if (scheme === "http" || scheme === "https") {
			throw new TypeErrorCtor("remote image URLs are not supported in tool outputs. Pass a base64 data URI instead");
		}
		const comma = url.indexOf(",");
		const header = comma === -1 ? [] : url.slice(colon + 1, comma).split(";");
		if (scheme !== "data" || comma === -1 || header.slice(1).every((part) => part.toLowerCase() !== "base64")) {
			throw new TypeErrorCtor("invalid image output. Pass a base64 data URI instead");
		}
		if (!finished) send("image", 0, url.slice(comma + 1), header[0] || "application/octet-stream");
	}

	function exit() {
		let writesJson;
		try {
			writesJson = serializeWrites();
		} catch (error) {
			done(false, describeError(error));
			throw EXIT;
		}
		done(true, undefined, writesJson);
		throw EXIT;
	}

	const console = objectCreate(null);
	const levels = ["log", "info", "warn", "error", "debug"];
	for (let i = 0; i < levels.length; i++) {
		console[levels[i]] = (...args) => {
			if (finished) return;
			let line = "";
			for (let j = 0; j < args.length; j++) line += (j === 0 ? "" : " ") + format(args[j]);
			send("text", 0, line);
		};
	}
	freeze(console);

	defineProperty(globalThis, "tools", { value: tools, enumerable: true });
	defineProperty(globalThis, "ALL_TOOLS", { value: allTools, enumerable: true });
	defineProperty(globalThis, "console", { value: console, enumerable: true });
	defineProperty(globalThis, "text", { value: text, enumerable: true });
	defineProperty(globalThis, "image", { value: image, enumerable: true });
	defineProperty(globalThis, "exit", { value: exit, enumerable: true });
	defineProperty(globalThis, "store", { value: store, enumerable: true });
	defineProperty(globalThis, "load", { value: load, enumerable: true });

	return {
		settle(id, ok, hasPayload, payload) {
			const entry = pending[id];
			if (entry === undefined) return;
			delete pending[id];
			pendingCount--;
			if (!ok) {
				entry.reject(new ErrorCtor(payload));
				return;
			}
			let value;
			try {
				value = hasPayload ? parse(payload) : undefined;
			} catch (error) {
				entry.reject(error);
				return;
			}
			entry.resolve(value);
		},
		run(fn) {
			let promise;
			try {
				promise = fn(tools, console);
			} catch (error) {
				if (error !== EXIT) done(false, describeError(error));
				return;
			}
			apply(promiseThen, promise, [
				(value) => {
					let json;
					let writesJson;
					try {
						json = serialize(value);
						writesJson = serializeWrites();
					} catch (error) {
						done(false, describeError(error));
						return;
					}
					done(true, json, writesJson);
				},
				(error) => {
					if (error !== EXIT) done(false, describeError(error));
				},
			]);
		},
		stalled() {
			if (finished) return "finished";
			if (pendingCount > 0) return "waiting";
			done(
				false,
				stringify({
					name: "Error",
					message:
						"The script is waiting on a promise that can never settle: no tool call is pending, and timers do not exist here.",
				}),
			);
			return "stalled";
		},
	};
})
