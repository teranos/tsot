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
wget https://amazoncloudwatch-agent.s3.amazonaws.com/ubuntu/$(dpkg --print-architecture)/latest/amazon-cloudwatch-agent.deb
sudo dpkg -i amazon-cloudwatch-agent.deb
rm amazon-cloudwatch-agent.deb
```

### 5. Deploy the relay binary

The relay is `roam/relayers/`, a Rust crate. Cross-compile from a
dev machine; the box is too small to build cargo. See
`roam/relay/relayers.md` for the deploy mechanics. Identity
(`ROAM_RELAY_IDENTITY_SECRET`) and AWS env are wired through the
existing `/etc/systemd/system/roam-relay.service` unit.

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
consecutive minutes.

```
aws cloudwatch get-metric-statistics --namespace CWAgent --metric-name mem_used_percent --dimensions Name=InstanceName,Value=roam-relay-eu-2 --start-time $(date -u -d '1 hour ago' -Iseconds) --end-time $(date -u -Iseconds) --period 60 --statistics Average
aws logs tail /roam/relay/journal --since 1h
```

`systemctl restart roam-relay` is the immediate mitigation;
`MemoryMax=400M` in the unit means the kernel kills before the box
wedges, and `Restart=always` brings it back.

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
