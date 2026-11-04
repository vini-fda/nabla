#!/usr/bin/env bash
#
# Installs or upgrades nabla-server as a systemd service, from a GitHub Release. Run it as root on
# the server, or from your own computer, which keeps the script out of the server's disk:
#
#   ssh root@203.0.113.7 'bash -s -- --version v0.28.0 --sha256 HASH --host 203.0.113.7' \
#     < deploy/install.sh
#
# HASH is the line for nabla-server-VERSION-x86_64-unknown-linux-gnu.tar.gz in the release's
# SHA256SUMS.txt. It is required so that a deployment says exactly which bytes it runs.
#
# Running it again is safe. It keeps the password, and restarts the server only if something
# changed, since a restart means a new certificate and so a new address. To upgrade or roll back,
# run it with another version. A version that fails to start is rolled back.
#
# It does not change the provider's firewall, which must allow the port over UDP.

set -euo pipefail

readonly REPO=vini-fda/nabla
readonly TARGET=x86_64-unknown-linux-gnu
readonly OLDEST_VERSION=v0.28.0
readonly ROOT=/opt/nabla
readonly CONFIG=/etc/nabla
readonly PASSWORD_FILE=$CONFIG/password
readonly UNIT_FILE=/etc/systemd/system/nabla-server.service
readonly ADDRESS_TOOL=/usr/local/bin/nabla-address
readonly SERVICE_USER=nabla

version=
sha256=
host=
port=4433
world=planet-moon
max_pilots=16

usage() {
  cat <<EOF
usage: install.sh --version vX.Y.Z --sha256 HASH --host HOST [options]

  --version vX.Y.Z   The release to install
  --sha256 HASH      The checksum of its server archive, from the release's SHA256SUMS.txt
  --host HOST        The server's public IP address or name, which pilots connect to
  --port N           The UDP port, from 1024 up (default: $port)
  --world NAME       vacuum, planet, planet-moon, or a world file (default: $world)
  --max-pilots N     How many ships may fly at once, from 1 to 16 (default: $max_pilots)
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

# Everything is in a function so that bash has read the whole script before running any of it: with
# `bash -s`, the script arrives on stdin, where a command that reads stdin could swallow the rest.
main() {
  while (($# > 0)); do
    case $1 in
      -h | --help)
        usage
        exit 0
        ;;
      --version | --sha256 | --host | --port | --world | --max-pilots)
        (($# >= 2)) || die "$1 needs a value"
        case $1 in
          --version) version=$2 ;;
          --sha256) sha256=$2 ;;
          --host) host=$2 ;;
          --port) port=$2 ;;
          --world) world=$2 ;;
          --max-pilots) max_pilots=$2 ;;
        esac
        shift 2
        ;;
      *)
        usage >&2
        die "unknown option: $1"
        ;;
    esac
  done

  # These values end up in a unit file and a script, so only harmless characters get through.
  [[ $version =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version must look like v0.28.0"
  # Older servers ignore flags they don't know, so they would run with no password at all.
  [[ $(printf '%s\n' "$OLDEST_VERSION" "$version" | sort -V | head -n 1) == "$OLDEST_VERSION" ]] ||
    die "--version must be $OLDEST_VERSION or newer, the first server with --password-file"
  [[ $sha256 =~ ^[0-9a-f]{64}$ ]] || die "--sha256 must be 64 lowercase hexadecimal digits"
  [[ $host =~ ^[A-Za-z0-9.:-]+$ ]] || die "--host must be an IP address or a name"
  [[ $port =~ ^[0-9]{4,5}$ ]] && ((port >= 1024 && port <= 65535)) ||
    die "--port must be from 1024 to 65535"
  [[ $world =~ ^[A-Za-z0-9._/-]+$ ]] || die "--world has characters it cannot have"
  [[ $max_pilots =~ ^[0-9]{1,2}$ ]] && ((max_pilots >= 1 && max_pilots <= 16)) ||
    die "--max-pilots must be from 1 to 16"

  ((EUID == 0)) || die "run this as root"
  [[ -d /run/systemd/system ]] || die "this needs a system running systemd"
  [[ $(uname -m) == x86_64 ]] || die "releases have no Linux server for $(uname -m) yet"

  missing=()
  for tool in curl tar sha256sum base64 journalctl; do
    command -v "$tool" >/dev/null || missing+=("$tool")
  done
  if ((${#missing[@]} > 0)); then
    command -v apt-get >/dev/null || die "please install: ${missing[*]}"
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      curl ca-certificates tar coreutils systemd >/dev/null
  fi

  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT

  # Download and check it, and see that it runs here, before anything on the system changes. An old
  # glibc, for one, fails at this point.
  name=nabla-server-$version-$TARGET
  echo "==> Downloading $name"
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 \
    --output "$work/server.tar.gz" \
    "https://github.com/$REPO/releases/download/$version/$name.tar.gz"
  echo "$sha256  $work/server.tar.gz" | sha256sum --check --quiet ||
    die "the download does not match --sha256"
  tar --no-same-owner -xzf "$work/server.tar.gz" -C "$work" "$name/nabla-server"
  "$work/$name/nabla-server" --help >/dev/null 2>&1 ||
    die "this version of nabla-server does not run on this system"

  changed=false

  if ! id "$SERVICE_USER" >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin \
      "$SERVICE_USER"
  fi

  install -d -m 755 "$CONFIG"
  new_password=
  if [[ ! -e $PASSWORD_FILE ]]; then
    new_password=$(head -c 24 /dev/urandom | base64)
    install -m 600 /dev/null "$PASSWORD_FILE"
    printf '%s\n' "$new_password" >"$PASSWORD_FILE"
    changed=true
  fi
  # The server runs as this user, and warns about a password file that anyone else can read.
  chown "$SERVICE_USER:" "$PASSWORD_FILE"
  chmod 600 "$PASSWORD_FILE"

  release=$ROOT/releases/$version
  install -d -m 755 "$release"
  if ! cmp --silent "$work/$name/nabla-server" "$release/nabla-server"; then
    install -m 755 "$work/$name/nabla-server" "$release/nabla-server"
    changed=true
  fi

  # Listen on IPv6 too when the system has it, or a name with an AAAA record stays on Connecting.
  # With `bindv6only` at 0, the Linux default, this socket also takes IPv4.
  bind=0.0.0.0:$port
  if [[ -n $(cat /proc/net/if_inet6 2>/dev/null) &&
    $(cat /proc/sys/net/ipv6/bindv6only 2>/dev/null) == 0 ]]; then
    bind=[::]:$port
  fi

  cat >"$work/unit" <<EOF
[Unit]
Description=Nabla multiplayer server
After=network-online.target
Wants=network-online.target

[Service]
User=$SERVICE_USER
ExecStart=$ROOT/current/nabla-server --bind $bind --name $host --world $world --max-pilots $max_pilots --password-file $PASSWORD_FILE
Restart=always
RestartSec=5
# A self-signed certificate lasts 14 days, so make a new one in time. It changes the address:
# nabla-address prints the current one.
RuntimeMaxSec=13d
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectKernelLogs=true
ProtectControlGroups=true
RestrictNamespaces=true
RestrictSUIDSGID=true
LockPersonality=true
MemoryDenyWriteExecute=true
SystemCallArchitectures=native
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6
MemoryMax=256M

[Install]
WantedBy=multi-user.target
EOF

  if ! cmp --silent "$work/unit" "$UNIT_FILE"; then
    install -m 644 "$work/unit" "$UNIT_FILE"
    systemctl daemon-reload
    changed=true
  fi

  # The server logs its address with `localhost` when it listens on every interface.
  public_host=$host
  [[ $host == *:* ]] && public_host=[$host]
  cat >"$work/address" <<EOF
#!/bin/sh
# Prints the address pilots connect to, from the log of the running server.
invocation=\$(systemctl show --property=InvocationID --value nabla-server)
[ -n "\$invocation" ] || exit 1
address=\$(journalctl --output=cat "_SYSTEMD_INVOCATION_ID=\$invocation" |
  grep -o 'https://localhost:[^ ]*' | tail -n 1)
[ -n "\$address" ] || exit 1
echo "\$address" | sed 's|^https://localhost:|https://$public_host:|'
EOF
  install -m 755 "$work/address" "$ADDRESS_TOOL"

  # The new version goes live here, by replacing the link in one step.
  previous=$(readlink "$ROOT/current" 2>/dev/null || true)
  switch_to() {
    ln -s "$1" "$ROOT/current.new"
    mv --no-target-directory "$ROOT/current.new" "$ROOT/current"
  }
  if [[ $previous != "$release" ]]; then
    switch_to "$release"
    changed=true
  fi

  if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q '^Status: active'; then
    ufw allow "$port/udp" >/dev/null
  fi

  # Waits for the running server to log its address, which is the sign that it is listening.
  wait_until_listening() {
    local attempt
    for attempt in {1..15}; do
      "$ADDRESS_TOOL" >/dev/null 2>&1 && systemctl is-active --quiet nabla-server && return 0
      sleep 1
    done
    return 1
  }

  systemctl enable --quiet nabla-server
  if $changed || ! systemctl is-active --quiet nabla-server; then
    echo "==> Starting $version"
    systemctl restart nabla-server
  fi

  if ! wait_until_listening; then
    journalctl --unit=nabla-server --lines=20 --no-pager >&2 || true
    if [[ -n $previous && $previous != "$release" ]]; then
      echo "==> $version did not start, so going back to $previous" >&2
      switch_to "$previous"
      systemctl restart nabla-server
    fi
    die "the server did not start"
  fi

  echo
  echo "nabla-server $version is running."
  echo
  echo "Address:  $("$ADDRESS_TOOL")"
  if [[ -n $new_password ]]; then
    echo "Password: $new_password"
    echo "          (shown this once; it is also in $PASSWORD_FILE)"
  else
    echo "Password: unchanged, in $PASSWORD_FILE"
  fi
  echo
  echo "The address changes on every restart, and at least every 13 days. To print it: nabla-address"
  echo
  echo "Check that the provider's firewall allows $port/udp, then connect from another computer."
}

main "$@" </dev/null
