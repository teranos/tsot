# Identity persistence for the libp2p relay.
#
# The relay's Ed25519 private key derives its peer-id, which derives
# its multiaddr. Today the key lives only in `./relay/.peer-key` on
# the box's filesystem — a box recreation generates a new key, which
# changes the peer-id, which breaks every client's bootstrap
# multiaddr. This is the root cause of the "peer-id changed, must
# update dist/relay-multiaddr.txt every box rebuild" problem.
#
# Fix: store the protobuf-encoded private key in AWS Secrets Manager.
# On startup the relay GET it; on first ever start (or if it doesn't
# exist) the relay generates one and PUT it. Box recreation then
# fetches the same key and the peer-id stays stable.
#
# Lightsail does NOT support EC2-style IAM instance profiles. The box
# authenticates as a dedicated IAM user using long-lived access keys
# the operator copies to /etc/systemd/system/roam-relay.service
# `Environment=` directives once per rotation. Access key + secret
# are surfaced as sensitive tofu outputs.

resource "aws_secretsmanager_secret" "relay_identity" {
  name = "roam/relay/identity"

  description = "Protobuf-encoded Ed25519 private key for the libp2p relay. Determines the relay's peer-id; rotating this changes the multiaddr clients bootstrap against."

  # Short recovery window — if we lose the secret, the relay can
  # regenerate one on next start (with new peer-id). Not catastrophic;
  # 7 days is enough to spot a mistaken delete.
  recovery_window_in_days = 7
}

resource "aws_iam_user" "relay" {
  name = "roam-relay"
  path = "/service/"
}

resource "aws_iam_user_policy" "relay_identity_rw" {
  name = "relay-identity-rw"
  user = aws_iam_user.relay.name

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Sid    = "RelayIdentitySecretAccess"
      Effect = "Allow"
      Action = [
        "secretsmanager:GetSecretValue",
        "secretsmanager:PutSecretValue",
        "secretsmanager:DescribeSecret",
      ]
      Resource = aws_secretsmanager_secret.relay_identity.arn
    }]
  })
}

resource "aws_iam_access_key" "relay" {
  user = aws_iam_user.relay.name
}

output "relay_access_key_id" {
  description = "Access key ID for the roam-relay IAM user. Copy into the box's systemd unit as ROAM_AWS_ACCESS_KEY_ID."
  value       = aws_iam_access_key.relay.id
  sensitive   = true
}

output "relay_secret_access_key" {
  description = "Secret access key for the roam-relay IAM user. Copy into the box's systemd unit as ROAM_AWS_SECRET_ACCESS_KEY. Available exactly once; rotate by replacing aws_iam_access_key.relay."
  value       = aws_iam_access_key.relay.secret
  sensitive   = true
}

output "relay_identity_secret_id" {
  description = "Secrets Manager secret ID the relay reads its identity from. Pass as ROAM_RELAY_IDENTITY_SECRET to the relay process."
  value       = aws_secretsmanager_secret.relay_identity.id
}
