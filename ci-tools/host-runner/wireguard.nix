# WireGuard client configuration for a host runner.
#
# The host runner dials out to a WireGuard gateway so it can be reached over
# the tunnel even though it sits behind NAT on the lab network.
#
# Usage from a hostrunner module:
#
#   imports = [ (import ../wireguard.nix { address = "10.100.0.2/24"; }) ];
#
# Gateway endpoint and public key default to `./secrets/wireguard.nix` (which
# is gitignored) so deployment-specific infrastructure details stay out of
# version control:
#
#   {
#     publicKey = "<gateway-public-key>";
#     endpoint = "<gateway-host-or-ip>:51820";
#   }
#
# The host runner's own private key is generated on-device and is intentionally
# NOT part of this repository or the nix store:
#
#   $ mkdir -p -m 700 ~/.wireguard
#   $ nix shell nixpkgs#wireguard-tools -c sh -c 'umask 077; wg genkey > ~/.wireguard/privkey'
#   $ nix shell nixpkgs#wireguard-tools -c sh -c 'wg pubkey < ~/.wireguard/privkey'
#
# Register the printed public key as a peer on the gateway.
{
  # Tunnel address of this host runner (CIDR, e.g. "10.100.0.2/24").
  address,
  # Where the on-device private key lives.
  privateKeyFile ? "/home/hostrunner/.wireguard/privkey",
  # Gateway details. Loaded from ./secrets/wireguard.nix unless passed explicitly.
  gatewayPublicKey ? (import ./secrets/wireguard.nix).publicKey,
  gatewayEndpoint ? (import ./secrets/wireguard.nix).endpoint,
  tunnelSubnet ? "10.100.0.0/24",
}:
{ pkgs, ... }:
{
  environment.systemPackages = [ pkgs.wireguard-tools ];

  networking.wireguard.interfaces.wg0 = {
    ips = [ address ];
    inherit privateKeyFile;
    peers = [
      {
        publicKey = gatewayPublicKey;
        # Only route the tunnel subnet through the gateway. The host runner must
        # keep using its normal default route to reach GitHub for CI jobs.
        allowedIPs = [ tunnelSubnet ];
        endpoint = gatewayEndpoint;
        # We are behind NAT; keep the mapping alive so the gateway can reach us.
        persistentKeepalive = 25;
      }
    ];
  };
}
