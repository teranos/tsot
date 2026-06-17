# infra/RUNBOOK.md

Operational procedures for the deployed roam.sbvh.nl + relay.sbvh.nl
stack. Each procedure is something a human runs once (or rarely);
nothing in here is wired into CI.

---

## Provisioning a fresh relay box

Order matters: tofu first, then the box receives credentials, then the
relay starts. Out-of-order means the relay either generates a doomed
local key or fails to authenticate.

### 1. Apply tofu (if not already current)

```
cd infra
tofu apply
```

Confirms `aws_secretsmanager_secret.relay_identity`,
`aws_iam_user.relay`, `aws_iam_access_key.relay`,
`aws_cloudwatch_log_group.relay_journal`,
`aws_cloudwatch_log_group.relay_cwagent`,
`aws_ssm_parameter.cwagent_config`, and the two metric alarms exist.

### 2. Read the sensitive outputs

```
tofu output -raw relay_access_key_id
tofu output -raw relay_secret_access_key
tofu output -raw relay_identity_secret_id
tofu output -raw relay_cwagent_config_param
```

The secret access key is shown exactly once per `aws_iam_access_key`
resource — tofu re-reads it from state, but if you ever
`terraform state rm aws_iam_access_key.relay`, the value is lost and
the resource must be replaced. To rotate, taint and re-apply:

```
tofu taint aws_iam_access_key.relay
tofu apply
```

### 3. SSH to the box

```
ssh -i ~/.ssh/lightsail-roam-relay.pem ubuntu@<box-ip>
```

Box IP is in `tofu output relay_origin_domain_in_use` (resolves via
`origin-relay.sbvh.nl`).

### 4. Install AWS CLI + CloudWatch agent

```bash
sudo apt update
sudo apt install -y awscli

# CloudWatch agent (Amazon's .deb, pinned URL):
wget https://amazoncloudwatch-agent.s3.amazonaws.com/ubuntu/arm64/latest/amazon-cloudwatch-agent.deb
sudo dpkg -i amazon-cloudwatch-agent.deb
rm amazon-cloudwatch-agent.deb
```

(`arm64` if `dpkg --print-architecture` says `arm64`; `amd64`
otherwise.)

### 5. Write the relay's systemd unit

`/etc/systemd/system/roam-relay.service`:

```ini
[Unit]
Description=roam libp2p relay
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/roam
ExecStart=/home/ubuntu/.bun/bin/bun run relay/relay.ts
Restart=always
RestartSec=5

# Logging through journald → picked up by CloudWatch agent's syslog
# tail (see /opt/aws/amazon-cloudwatch-agent/etc/amazon-cloudwatch-agent.json).
StandardOutput=journal
StandardError=journal
SyslogIdentifier=roam-relay

# Memory ceiling. The wedge incident showed bun's RSS could grow
# until the kernel OOM-killed everything; 400 MiB on a 512 MiB box
# leaves headroom for the OS + the CloudWatch agent. systemd kills
# the unit if exceeded; Restart=always brings it back.
MemoryMax=400M

# Production env. Listen on all interfaces (CloudFront origins via
# origin-relay.sbvh.nl → public IP); announce the public wss
# multiaddr; skip the dist/ write (CloudFront serves dist/ from S3).
Environment=ROAM_RELAY_LISTEN_HOST=0.0.0.0
Environment=ROAM_RELAY_LISTEN_PORT=9001
Environment=ROAM_RELAY_ANNOUNCE=/dns4/relay.sbvh.nl/tcp/443/wss
Environment=ROAM_RELAY_WRITE_DIST=0

# Identity persistence (Secrets Manager). Stops box recreation from
# changing the peer-id; without these env vars set, the relay falls
# back to ./relay/.peer-key (local file) which is per-box.
Environment=ROAM_RELAY_IDENTITY_SECRET=<paste tofu output relay_identity_secret_id>
Environment=AWS_REGION=eu-central-1
Environment=AWS_ACCESS_KEY_ID=<paste tofu output relay_access_key_id>
Environment=AWS_SECRET_ACCESS_KEY=<paste tofu output relay_secret_access_key>

[Install]
WantedBy=multi-user.target
```

Then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now roam-relay
journalctl -u roam-relay -f
```

The first journal lines should include:

- `[relay] loaded identity from Secrets Manager (roam/relay/identity, N bytes)`
  (on subsequent starts), OR
- `[relay] secret unreadable as identity (...); generating new key and PUT-ing`
  (on the very first start where the secret was just created by tofu
  with no value yet).

### 6. Wire CloudWatch agent to its SSM config

```bash
sudo /opt/aws/amazon-cloudwatch-agent/bin/amazon-cloudwatch-agent-ctl \
  -a fetch-config \
  -m ec2 \
  -s \
  -c ssm:/roam/relay/cwagent/config
```

The agent reads its credentials from the same env vars used by the
relay — write them into
`/opt/aws/amazon-cloudwatch-agent/etc/common-config.toml`:

```
[credentials]
shared_credential_profile = "default"
```

…and populate `/root/.aws/credentials` (the agent runs as root):

```
[default]
aws_access_key_id = <paste tofu output relay_access_key_id>
aws_secret_access_key = <paste tofu output relay_secret_access_key>
region = eu-central-1
```

Verify metrics flowing within a minute:

```
aws cloudwatch list-metrics --namespace CWAgent --region eu-central-1 | grep roam
```

---

## Rotating the relay's AWS access key

The IAM access key has no hard expiry, but AWS recommends rotating
quarterly. To rotate:

```
cd infra
tofu taint aws_iam_access_key.relay
tofu apply
tofu output -raw relay_access_key_id
tofu output -raw relay_secret_access_key
```

Then SSH to the box, update `/etc/systemd/system/roam-relay.service`
and `/root/.aws/credentials` with the new key, and:

```
sudo systemctl daemon-reload
sudo systemctl restart roam-relay
sudo systemctl restart amazon-cloudwatch-agent
```

---

## Memory alarm fired — what to do

The alarm `roam-relay-memory-high` triggers at 80% memory for 5
consecutive minutes. Pre-OOM diagnostic procedure:

1. `aws cloudwatch get-metric-statistics --namespace CWAgent --metric-name mem_used_percent --dimensions Name=InstanceName,Value=roam-relay-eu-2 --start-time $(date -u -d '1 hour ago' -Iseconds) --end-time $(date -u -Iseconds) --period 60 --statistics Average`
2. `aws logs tail /roam/relay/journal --since 1h | grep -E 'error|warn|FATAL'`
3. SSH and run `procstat -p $(pgrep bun)` to see RSS growth pattern.
4. If RSS is growing monotonically with no plateau, that's a leak in
   the relay — restart buys time, but the upstream libp2p / gossipsub
   versions need investigating. Capture a heap dump (`bun --heap-snapshot`)
   before the restart.

`systemctl restart roam-relay` is the immediate mitigation;
`MemoryMax=400M` in the unit means the kernel will kill the process
before the box wedges, and `Restart=always` brings it back.

---

## CI deploy fails on `AssumeRoleWithWebIdentity`

The OIDC trust policy in `infra/cicd.tf` restricts to:

```
repo:teranos/tsot:ref:refs/heads/master
```

If the failure message references a different `sub` claim, either:

- The push was to a different branch — by design, only master deploys.
- The repo was renamed/moved — update `var.github_repo` in
  `terraform.tfvars` (or the default in `variables.tf`) and re-apply.

If the GitHub thumbprint in `aws_iam_openid_connect_provider.github`
becomes invalid (GitHub rotates rarely), pull the current value from
`https://token.actions.githubusercontent.com/.well-known/openid-configuration`
and update `thumbprint_list`.
