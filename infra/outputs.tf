output "game_url" {
  description = "Public HTTPS URL audiences open to play."
  value       = "https://${local.game_fqdn}/"
}

output "static_bucket" {
  description = "S3 bucket holding the game bundle. `aws s3 sync` the dist/ contents here."
  value       = aws_s3_bucket.static.id
}

output "static_distribution_id" {
  description = "CloudFront distribution ID for invalidations after a new dist/ upload."
  value       = aws_cloudfront_distribution.static.id
}

