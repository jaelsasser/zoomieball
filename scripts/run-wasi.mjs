import { readFile } from "node:fs/promises";
import process from "node:process";
import { WASI } from "node:wasi";

const [modulePath, ...moduleArguments] = process.argv.slice(2);

if (modulePath === undefined) {
  console.error("usage: node scripts/run-wasi.mjs <module.wasm> [arguments...]");
  process.exit(2);
}

const wasi = new WASI({
  version: "preview1",
  args: [modulePath, ...moduleArguments],
  env: {},
});
const module = await WebAssembly.compile(await readFile(modulePath));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
wasi.start(instance);
