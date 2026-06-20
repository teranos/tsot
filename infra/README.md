# infra/

OpenTofu config for roam's hosted deployment.

## Architecture

```
roam.sbvh.nl   ── A/AAAA alias ──► CloudFront distribution (static) ──► S3 bucket
relay.sbvh.nl  ── A/AAAA alias ──► CloudFront distribution (relay)  ──► Lightsail :9001
```

- **Static site** (`roam.sbvh.nl`): `dist/` bundled by `make wasm` is
  uploaded to S3, served through CloudFront with an ACM-issued cert.
- **Relay** (`relay.sbvh.nl`): the libp2p `roam/relayers/` Rust
  binary runs on a Lightsail nano. CloudFront terminates TLS (ACM cert) and
  forwards the WebSocket over plain HTTP on port 9001. The relay does
  not handle TLS itself.

## Files

| File | Purpose |
|---|---|
| `versions.tf` | OpenTofu + provider version pins |
| `main.tf` | Two AWS providers — `eu-central-1` (primary) and `us-east-1` alias (CloudFront ACM requirement) |
| `variables.tf` | Inputs (domain, subdomains, profile, relay origin) |
| `locals.tf` | FQDN composition |
| `route53.tf` | A/AAAA aliases + ACM DNS validation records |
| `acm.tf` | Two certs in us-east-1 |
| `s3.tf` | Private bucket + OAC + CF-only read policy |
| `cloudfront.tf` | Two distributions (static, relay) |
| `lightsail.tf` | Relay box instance (keypair + ports hand-managed; see file header) |
| `outputs.tf` | URLs, bucket name, distribution IDs |

## Lightsail

The relay instance `roam-relay-eu-2` (Ubuntu 24.04 nano in
`eu-central-1a`) is managed by tofu. Its keypair and firewall ports
are NOT — the AWS provider v6.50.0 does not support `tofu import` for
`aws_lightsail_key_pair` or `aws_lightsail_instance_public_ports`, and
recreating them would destroy SSH access / firewall state. See the
file header in `lightsail.tf` for the hand-managed sidecar inventory.

The relay's public IP is written to `origin-relay.sbvh.nl` by the
`aws_route53_record.origin_relay` resource in `route53.tf`, sourced
from `aws_lightsail_instance.relay.public_ip_address` — so a
stop/start of the box automatically updates DNS on the next apply.

A static IP would remove the stop/start IP churn but adds cost and
complexity. Deferred until the dynamic-IP-plus-Route-53 pattern shows
operational pain.

## Workflow

```
# Initial deploy (after AWS credentials work):
tofu init
tofu plan
tofu apply

# After uploading new dist/ to S3:
aws s3 sync ../roam/dist s3://$(tofu output -raw static_bucket) --delete
aws cloudfront create-invalidation \
    --distribution-id $(tofu output -raw static_distribution_id) \
    --paths '/*'

# After Lightsail comes up and origin-relay.sbvh.nl is set:
tofu apply -var "relay_origin_domain=origin-relay.sbvh.nl"
```

## What's not in here yet

- Lightsail keypair + firewall ports (provider can't import — see `lightsail.tf` header)
- Static bundle upload (done out-of-band via `aws s3 sync`)
- Cache invalidation after a new deploy (also out-of-band)
- Relay identity persistence (peer-id today survives only because the
  box's `.peer-key` file does — move to Secrets Manager so a recreated
  box keeps its multiaddr)
- CloudWatch agent + alarms on the Lightsail box

## Deferred

- **CI for static deploy.** GitHub Actions workflow that runs
  `make wasm` on push, then `aws s3 sync ... && create-invalidation`.
- **Observability.** CloudWatch agent on the relay box pushing memory
  / CPU / process RSS at 1-minute resolution, journald → CloudWatch
  Logs. Currently only an on-box shell script writing to local disk —
  there's no remote read path, which means any future wedge is again
  debugged with `aws lightsail get-instance-snapshot` forensics.
