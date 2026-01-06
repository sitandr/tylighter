# A minimal unofficial client-side Typst compiler

I have been looking for a long time towards `typst.ts`, but it was too complex for me to comprehend properly. So I've made a very simple Typst "Web Engine" myself.

Currently, it supports (from js via WASM):

- Compiling Typst to _html_ with WASM on client
- Setting any path for "file" your code might need with any data
- Returning very simple error messages (return byte numbers instead of line numbers for now)
- Set _memoization cache size_ (default: 10)
- Set JS function for _requesting additional files from you_
- Get metadata from compilation (currently only one with a fixed label `interact-var` is supported)
- For rendering SVG currently uses built-in fonts only

There is a lot of things to improve and implement, but the goal is to keep the design simple. And moreover, this allows us doing pretty sick things already! (I will later insert a link here)

To use it, simply download web bundle from releases, and import the code:

```js
import init, { js_recompile, set_request_f, update_file, set_cache_size } from '.../typst-interactive.js';
```

A very brief reference:


- `init`: `await init()` is required before any usage due to need to initialize the WebAssembly.
- After initialization, you should set `set_request_f((package_name: string, file_name: string) => Option<Uint8Array>)` for WebAssembly to be able to request you a file from a given package (may be empty) and a name. You may return null for it to panic first, but then you updating the file and call the recompiling manually.
- If compiler requested a file, it now knows the way it needs to load it (raw binary, decoded to utf-8 or both). The main file, `main.typ`, is always known. After that, you may update the file contents with `update_file(package_name: string, file_name: string, data: Uint8Array)`.
  _if it sounds pretty bad for speed, it is, that leads to increased loading times while compiler requests all files it needs; I'm planning to improve it_
- `js_recompile()` is a function that compiles a document with all file knowledge WebAssembly currently has, and returns a Results with raw html string and metadata string if succeeded, and an error span with `message` and `hints` otherwise.
- There is also a function `set_cache_size(n)` to set the number of last iterations to memoize.
