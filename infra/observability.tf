# Observability for the libp2p relay.
#
# Approach: the relay publishes its own metrics inline via
# `cloudwatch:PutMetricData` from the bun process. No CloudWatch
# agent, no SSM-stored agent config, no second credential surface on
# the box. The same IAM user that owns the identity secret gains
# `PutMetricData` (scoped to the `CWAgent` namespace via condition).
#
# Logs stay on the box (journald) for now. The CW Log Groups below
# are reserved infrastructure — provisioned so a future log-shipper
# (vector, fluent-bit, or a few lines in relay.ts using the
# CloudWatch Logs SDK) has a destination without re-running tofu.
# Alarms fire on metrics the relay publishes itself.

resource "aws_cloudwatch_log_group" "relay_journal" {
  # Reserved for future log shipping from the relay process or a
  # log-shipper sidecar. Empty today.
  name              = "/roam/relay/journal"
  retention_in_days = 30

  tags = {
    Component = "relay"
    Status    = "reserved-for-future-log-shipping"
  }
}

resource "aws_iam_user_policy" "relay_observability" {
  name = "relay-observability"
  user = aws_iam_user.relay.name

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "CloudWatchMetricsPut"
        Effect = "Allow"
        Action = [
          "cloudwatch:PutMetricData",
        ]
        Resource = "*"
        # PutMetricData doesn't support resource-level permissions in
        # IAM; constrain by namespace via condition. The relay
        # publishes under the `CWAgent` namespace so the existing
        # alarm dimensions (InstanceName=roam-relay-eu-2) match.
        Condition = {
          StringEquals = {
            "cloudwatch:namespace" = "CWAgent"
          }
        }
      },
      {
        # Reserved for when log shipping lands. Removing this Sid is
        # a one-line change once the shipper is written.
        Sid    = "CloudWatchLogsWrite"
        Effect = "Allow"
        Action = [
          "logs:CreateLogStream",
          "logs:PutLogEvents",
          "logs:DescribeLogStreams",
        ]
        Resource = [
          "${aws_cloudwatch_log_group.relay_journal.arn}:*",
        ]
      },
    ]
  })
}

# Memory alarm. The wedge incident's signature was bun's RSS climbing
# until oom-killer fired; 80% sustained for 5 minutes catches that
# pattern with time to investigate before the kernel responds. The
# relay publishes `mem_used_percent` (RSS / 512 MiB) every 60s.
resource "aws_cloudwatch_metric_alarm" "relay_memory_high" {
  alarm_name          = "roam-relay-memory-high"
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 5
  metric_name         = "mem_used_percent"
  namespace           = "CWAgent"
  period              = 60
  statistic           = "Average"
  threshold           = 80
  alarm_description   = "Relay memory above 80% for 5 consecutive minutes. Investigate before the kernel OOMs the box. Repeats of this pattern preceded the 2026-06 wedge."
  treat_missing_data  = "breaching"

  dimensions = {
    InstanceName = "roam-relay-eu-2"
  }
}

# Pubsub message rate dropping to zero is the silent-relay signal.
# Healthy relays gossip every few seconds when peers are connected;
# zero for 10 minutes with a non-zero peer count means something
# stopped routing.
resource "aws_cloudwatch_metric_alarm" "relay_pubsub_silent" {
  alarm_name          = "roam-relay-pubsub-silent"
  comparison_operator = "LessThanOrEqualToThreshold"
  evaluation_periods  = 10
  metric_name         = "relay_pubsub_msgs_per_sec"
  namespace           = "CWAgent"
  period              = 60
  statistic           = "Average"
  threshold           = 0
  alarm_description   = "Relay observed zero pubsub messages/sec for 10 consecutive minutes. Suspect: gossipsub stuck, peer mesh empty, or process wedged but TCP alive. Cross-check with relay_peer_count to distinguish 'no peers' from 'silent relay'."
  treat_missing_data  = "notBreaching"

  dimensions = {
    InstanceName = "roam-relay-eu-2"
  }
}

output "relay_journal_log_group" {
  description = "CloudWatch Log Group reserved for future log shipping (empty today)."
  value       = aws_cloudwatch_log_group.relay_journal.name
}
