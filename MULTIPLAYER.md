# Multiplayer

Nabla's online mode has pilots share a world on a server, from the native app or a browser. How it works is in `docs/multiplayer.typ`; this file is about running it.

## Hosting on a LAN

To fly with people on your own network, such as your family at home, run the server on one computer that stays on, and have everyone connect to it. You need no account and no port forwarding, and nothing is exposed to the internet.

The examples use `192.168.1.50` as the host's address. Use your own.

### 1. Get the server

Download `nabla-server` for Linux x86-64 or macOS on Apple silicon from the [releases](https://github.com/vini-fda/nabla/releases), or build it from the code with [rustup](https://rustup.rs) and a C compiler (on Linux, also `mold` and `clang`):

```sh
git clone https://github.com/vini-fda/nabla
cd nabla
cargo build --release -p nabla-server --bin nabla-server
cp target/release/nabla-server .
```

The worlds are built into the binary, so it is the only file you need.

### 2. Find the host's address

```sh
hostname -I                # Linux
ipconfig getifaddr en0     # macOS, on the usual network interface
```

Routers can hand out a different address over time. To keep the one you give your family, reserve it for this computer in the router's DHCP settings.

### 3. Start it

```sh
./nabla-server --bind 192.168.1.50:4433 --name 192.168.1.50
```

It logs the address to share, with the certificate's hash after the `#`:

```text
connect to https://192.168.1.50:4433#sha256=dea539c5...
```

Add `--world vacuum`, `planet` or a world file to fly elsewhere than the default planet and moon. [Running a server](README.md#running-a-server) lists all the flags.

The server listens on UDP, so a firewall on the host must allow that port:

```sh
sudo ufw allow 4433/udp    # Linux with ufw
```

With the macOS firewall on, allow incoming connections if it asks.

### 4. Connect

Send everyone the whole address, `#sha256=...` included. Then:

- **In the app:** choose Online on the title screen, paste the address, press Connect, pick a username in the lobby, and press Take this ship.
- **In a browser:** do the same in the [web app](https://vini-fda.github.io/nabla/). It needs WebTransport, which Chrome, Edge and Firefox have, and Safari from 26.4. If the browser asks to access devices on your local network, allow it.

Use the web app at that address and not a copy served from the host. Browsers only offer WebTransport on `https://` pages and on `localhost`, so a page opened from `http://192.168.1.50` cannot connect.

A username is 1 to 32 lowercase letters and digits, and no two pilots flying at once can share one. Each ship shows its pilot's name beside it, with a dot that is green while they are flying and gray when the app has lost focus, the connection has gone quiet, or they have touched nothing for 30 seconds.

The world comes from the server, so pilots need nothing but the address. Up to 16 can fly at once (`--max-pilots` lowers that), and a ship that goes silent for a minute leaves and frees its place.

### Keeping it running

The server makes a new certificate each time it starts, good for 14 days. That means a new hash, and so a new address to send around, on every restart. Browsers and the app both refuse a hashed certificate that lasts longer than 14 days, so the server must be restarted within that time.

To keep the same address across restarts, make the certificate yourself and renew it every 14 days:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout key.pem -out cert.pem -days 14 -subj "/CN=nabla" \
  -addext "subjectAltName=IP:192.168.1.50"
./nabla-server --bind 192.168.1.50:4433 --cert cert.pem --key key.pem
```

The hash then changes only when you renew.

### If it doesn't connect

| What you see | Why, and what to do |
|---|---|
| It stays on Connecting | Nothing answers at that address. Check the address and the port, that the server is running, and the host's firewall. Guest Wi-Fi networks and "client isolation" also keep devices apart. |
| `invalid peer certificate: UnknownIssuer` in the app, or `Opening handshake failed` in a browser | The hash is missing or out of date, usually from an address saved before a restart. Copy the address from the server's log again. A certificate longer than 14 days fails the same way. |
| The server is full | Every place is taken. Press Ask again when someone leaves. |
| The server turned this client away | The client and the server speak different versions of the protocol. Update whichever is older. |
| A name such as `host.local` stays on Connecting | The name can resolve to IPv6 first, and a server on an IPv4 address never hears it. Use the IP address, or start the server with `--bind '[::]:4433'`. |
