# Connecting to a Caliptra Host Runner over WireGuard

This guide walks through setting up WireGuard to SSH to a Caliptra Raspberry Pi
host runner (for example, `ssh hostrunner@10.100.0.2`). Expect it to take about
10 minutes plus however long it takes an administrator to register your key.

## What you are connecting to

```
 your laptop  ──UDP 51820──▶  WireGuard gateway  ◀──────  host runner
 10.100.0.1xx                 <gateway-endpoint>          10.100.0.x
                               10.100.0.1              (Raspberry Pi, NixOS)
```

Host runners sit behind NAT on a lab network, so they dial out to a publicly
reachable WireGuard gateway. You connect to the same gateway, and it forwards
traffic between you and the Pi. Only the `10.100.0.0/24` tunnel subnet is routed
through WireGuard; the rest of your network traffic is unaffected.

| Name | Tunnel address | Notes |
|---|---|---|
| gateway | `10.100.0.1` | Publicly reachable WireGuard hub |
| host runner | `10.100.0.x` (e.g. `10.100.0.2`) | SSH user `hostrunner` |
| you | `10.100.0.1xx` | Assigned by the gateway admin |

## Step 1: Install WireGuard

| OS | Command |
|---|---|
| Debian / Ubuntu | `sudo apt install wireguard` |
| Fedora | `sudo dnf install wireguard-tools` |
| macOS (CLI) | `brew install wireguard-tools` |
| macOS (GUI) | [WireGuard](https://apps.apple.com/us/app/wireguard/id1451685025) from the App Store |
| NixOS | add `pkgs.wireguard-tools` to `environment.systemPackages` |
| Windows | [wireguard.com/install](https://www.wireguard.com/install/) |

## Step 2: Generate your key pair

Your private key never leaves your machine. You only share the public key.

```bash
umask 077
wg genkey | tee ~/wg-caliptra.key | wg pubkey > ~/wg-caliptra.pub
cat ~/wg-caliptra.pub
```

You will get a 44-character base64 string ending in `=`, for example
`7Kq2hM0b4Xf9T1nLwYpR8cVd3sZaUeJiGoHxNtBmFyA=`.

## Step 3: Request access

Send the gateway administrator the following:

1. Your WireGuard **public** key from Step 2.
2. Your SSH **public** key (`cat ~/.ssh/id_ed25519.pub`), if you do not already
   have SSH access to the host runners.
3. A short description of the machine, e.g. "alice laptop".

The administrator will reply with:

- Your assigned tunnel address (e.g. `10.100.0.100`)
- The gateway's public key (`<gateway-public-key>`)
- The gateway's endpoint (`<gateway-host-or-ip>:51820`)
- The target host runner's tunnel address (e.g. `10.100.0.2`)

Wait for those details before continuing.

## Step 4: Write the client config

Replace `10.100.0.100`, `<gateway-public-key>`, and `<gateway-endpoint>` with
the values from the administrator.

```bash
sudo mkdir -p /etc/wireguard
sudo tee /etc/wireguard/caliptra.conf > /dev/null <<EOF
[Interface]
Address = 10.100.0.100/24
PrivateKey = $(cat ~/wg-caliptra.key)

[Peer]
PublicKey = <gateway-public-key>
Endpoint = <gateway-endpoint>
AllowedIPs = 10.100.0.0/24
PersistentKeepalive = 25
EOF
sudo chmod 600 /etc/wireguard/caliptra.conf
rm ~/wg-caliptra.key   # the private key now lives only in the config
```

**macOS / Windows GUI app:** choose *Import tunnel(s) from file* and select a
file with the same contents, or *Add empty tunnel* and paste them in. Note the
GUI app generates its own key pair when you add an empty tunnel; if you use
that, send the administrator the public key it shows instead of the one from
Step 2.

## Step 5: Bring the tunnel up

```bash
sudo wg-quick up caliptra
```

Check it:

```bash
sudo wg show
#   peer: <gateway-public-key>
#     endpoint: <gateway-endpoint>
#     latest handshake: 3 seconds ago      <-- this line must be present
#     transfer: 1.2 KiB received, 2.4 KiB sent

ping -c 3 10.100.0.1   # gateway
ping -c 3 10.100.0.2   # host runner
```

If both pings succeed you are done with the network side.

To bring it down: `sudo wg-quick down caliptra`.
To start on boot (Linux): `sudo systemctl enable --now wg-quick@caliptra`.

## Step 6: SSH to the host runner

```bash
ssh hostrunner@10.100.0.2
```

Optional `~/.ssh/config` entry:

```
Host caliptra-hostrunner-wg
  Hostname 10.100.0.2
  User hostrunner
  IdentityFile ~/.ssh/id_ed25519
  IdentitiesOnly yes
```

Then just `ssh caliptra-hostrunner-wg`. Anything that takes an SSH host works
the same way, e.g. `scp`, `rsync`, or
`nixos-rebuild --target-host caliptra-hostrunner-wg`.

## Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `sudo wg show` has no `latest handshake` line | The admin has not added your key yet, or the key they added does not match. Run `sudo wg show caliptra public-key` and compare with what you sent. |
| Handshake OK, `ping 10.100.0.1` works, `ping 10.100.0.2` fails | The Pi's side of the tunnel is down. Tell the admin; nothing to fix on your end. |
| Works for a few minutes, then stops | `PersistentKeepalive = 25` is missing from your `[Peer]` section. |
| `Permission denied (publickey)` on SSH | Your SSH public key is not authorized on the Pi yet. Send it to the admin (Step 3). |
| Tunnel never comes up on a hotel / guest / corporate network | Outbound UDP to the gateway endpoint (port `51820`) is being blocked. Try a different network or tether to your phone. |
| `wg-quick: 'caliptra' already exists` | It is already up. `sudo wg-quick down caliptra` first. |

## Etiquette

- The Pi runs live CI jobs for the FPGA boards attached to it. Do not stop or
  restart the `caliptra-*` systemd services unless you own that board.
- Do not share your private key or your config file with anyone else. Each
  person gets their own peer entry so access can be revoked individually.
- Tell the admin when you no longer need access so your peer can be removed.

---

## Appendix: for the administrator

### Standing up a gateway

Any small Linux host with a static public IP and inbound UDP `51820` open can
serve as the gateway:

```bash
sudo apt install wireguard wireguard-tools
echo 'net.ipv4.ip_forward=1' | sudo tee /etc/sysctl.d/99-wireguard.conf
sudo sysctl -p /etc/sysctl.d/99-wireguard.conf

sudo mkdir -p -m 700 /etc/wireguard
sudo sh -c 'umask 077; wg genkey | tee /etc/wireguard/privkey | wg pubkey > /etc/wireguard/pubkey'
sudo tee /etc/wireguard/wg0.conf > /dev/null <<EOF
[Interface]
Address = 10.100.0.1/24
ListenPort = 51820
PrivateKey = $(sudo cat /etc/wireguard/privkey)
EOF
sudo chmod 600 /etc/wireguard/wg0.conf
sudo systemctl enable --now wg-quick@wg0
```

Record `/etc/wireguard/pubkey` and `<public-ip>:51820` in
`ci-tools/host-runner/secrets/wireguard.nix` on the machine you deploy NixOS
from.

### Registering or revoking a client peer

```bash
ssh <gateway-host>

sudo wg show                               # find the next free 10.100.0.1xx
sudo tee -a /etc/wireguard/wg0.conf > /dev/null <<'EOF'

# alice (laptop)
[Peer]
PublicKey = <their-public-key>
AllowedIPs = 10.100.0.100/32
EOF
sudo systemctl reload wg-quick@wg0         # wg syncconf; existing peers stay up
sudo wg show                               # confirm the new peer is listed
```

Reply to the requester with their tunnel address, the gateway public key, and
the gateway endpoint. If they also need SSH access, add their SSH public key to
`ci-tools/host-runner/ssh_keys.nix` and redeploy the host runner.

To revoke access, remove their `[Peer]` block from `/etc/wireguard/wg0.conf`
and run `sudo systemctl reload wg-quick@wg0`.
