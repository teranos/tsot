output "game_url" {
  description = "Public HTTPS URL audiences open to play."
  value       = "https://${local.game_fqdn}/"
}

output "relay_wss_url" {
  description = "wss:// URL the bridge dials. Already substituted into dist/relay-multiaddr.txt."
  value       = "wss://${local.relay_fqdn}/"
}

output "static_bucket" {
  description = "S3 bucket holding the game bundle. `aws s3 sync` the dist/ contents here."
  value       = aws_s3_bucket.static.id
}

output "static_distribution_id" {
  description = "CloudFront distribution ID for invalidations after a new dist/ upload."
  value       = aws_cloudfront_distribution.static.id
}

output "relay_distribution_id" {
  description = "CloudFront distribution ID for the relay."
  value       = aws_cloudfront_distribution.relay.id
}

output "relay_origin_domain_in_use" {
  description = "Current value of `relay_origin_domain` — the CloudFront origin for the relay. Update via tofu apply once Lightsail has a stable address."
  value       = var.relay_origin_domain
}
