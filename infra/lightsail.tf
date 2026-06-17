# Lightsail surface for the libp2p relay.
#
# Pragmatic scope: tofu manages the INSTANCE only. The keypair and the
# instance_public_ports resources are NOT in tofu because the AWS
# provider (v6.50.0) does not implement Import for either resource
# type — `tofu import aws_lightsail_key_pair.X` and
# `tofu import aws_lightsail_instance_public_ports.X` both error with
# `doesn't support import`. Recreating them in-place would
# destructively replace the keypair (invalidating every box authorized
# with it) and would not produce a stable plan for ports either.
#
# Hand-managed sidecars (NOT in tofu):
#   - `aws_lightsail_key_pair` "roam-relay"
#       Created via: aws lightsail create-key-pair --key-pair-name roam-relay
#       Private key saved by the operator to ~/.ssh/lightsail-roam-relay.pem.
#       AWS doesn't return the private key again; if it's lost, rotate
#       by creating a new keypair + replacing the instance.
#   - `aws_lightsail_instance_public_ports` for "roam-relay-eu-2"
#       Firewall managed via: aws lightsail put-instance-public-ports.
#       Current state: 22/tcp + 9001/tcp open to 0.0.0.0/0.
#
# If the AWS provider gains Import support for these resource types in
# a later release, fold them back in here and remove this preamble.

resource "aws_lightsail_instance" "relay" {
  name              = "roam-relay-eu-2"
  availability_zone = "eu-central-1a"
  blueprint_id      = "ubuntu_24_04"
  bundle_id         = "nano_3_0"
  key_pair_name     = "roam-relay"

  # Empty until Secrets Manager identity persistence lands (roadmap
  # item 3). Setting user_data forces instance recreation; we want
  # that exactly once, when the bootstrap script can re-acquire the
  # peer-id from Secrets Manager so the multiaddr survives rebuilds.
  user_data = ""

  lifecycle {
    ignore_changes = [
      user_data,
    ]
  }
}

# ---------------------------------------------------------------
# tofu import (run once on a fresh state; the instance is the only
# importable Lightsail resource):
#
#   tofu import aws_lightsail_instance.relay roam-relay-eu-2
#
# After import, `tofu plan` must report 0 changes against the live
# instance. Drift in name / blueprint / bundle / az / key_pair_name
# means either the file above or the live box was hand-modified —
# reconcile before any apply.
# ---------------------------------------------------------------
