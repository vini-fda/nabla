---
name: nabla-web
description: Launch the nabla web client dev server (trunk serve on port 8080), as defined by .claude/launch.json. Load when the user asks to start, run, or serve the web app, or to open it in a browser.
---

# nabla-web

Start the web dev server exactly as the launch configuration copied alongside
this file (`launch.json`, originally `.claude/launch.json`) defines:

- name: `nabla-web`
- runtime: `trunk serve --config crates/nabla-client/Trunk.toml --port 8080`
- run from the repository root; serves http://127.0.0.1:8080/

Steps:

1. Read `launch.json` in this directory to confirm the executable, args, and port.
2. If port 8080 already answers (`curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/`),
   report that the server is already running and stop.
3. Otherwise start it as a background process that outlives this command
   (`nohup trunk serve --config crates/nabla-client/Trunk.toml --port 8080 > /tmp/nabla-web.log 2>&1 &`,
   or the shell tool's background/session mode). `trunk` must be on PATH;
   install with `cargo install --locked trunk` if missing.
4. Poll http://127.0.0.1:8080/ until it returns HTTP 200 (the first wasm build
   can take a while), then report the URL — http://127.0.0.1:8080/#dev skips the
   service worker — and the log path /tmp/nabla-web.log.

To stop the server later, kill the `trunk serve` process (`pkill -f 'trunk serve'`).

The repo's canonical equivalent is `cargo xtask web-dev` (same server and default
port 8080, run from `crates/nabla-client`).
