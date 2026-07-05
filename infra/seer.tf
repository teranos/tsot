# seer.sbvh.nl — static site for the observability-first Bevy crate
# at `seer/`. Mirrors rave.tf — separate bucket + distribution + cert +
# role so seer's blast radius is scoped to itself. Serves both:
#   /            — seer.wasm + browser bootstrap (index.html)
#   /perf/<sha>/ — per-commit CI diagnostic HTML reports
#   /perf/latest — always the most recent commit's report

# ----- S3 bucket -----

resource "aws_s3_bucket" "seer_static" {
  bucket        = var.seer_bucket_name
  force_destroy = true
}

resource "aws_s3_bucket_public_access_block" "seer_static" {
  bucket = aws_s3_bucket.seer_static.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_cors_configuration" "seer_static" {
  bucket = aws_s3_bucket.seer_static.id

  cors_rule {
    allowed_origins = ["*"]
    allowed_methods = ["GET", "HEAD"]
    allowed_headers = ["*"]
    expose_headers  = []
    max_age_seconds = 3000
  }
}

resource "aws_cloudfront_origin_access_control" "seer_static" {
  name                              = "${var.seer_bucket_name}-oac"
  description                       = "OAC for seer static bucket"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_s3_bucket_policy" "seer_static_cf_read" {
  bucket = aws_s3_bucket.seer_static.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid       = "AllowCloudFrontServicePrincipalRead"
        Effect    = "Allow"
        Principal = { Service = "cloudfront.amazonaws.com" }
        Action    = ["s3:GetObject"]
        Resource  = "${aws_s3_bucket.seer_static.arn}/*"
        Condition = {
          StringEquals = {
            "AWS:SourceArn" = aws_cloudfront_distribution.seer_static.arn
          }
        }
      }
    ]
  })
}

# ----- ACM certificate (us-east-1 — CloudFront requirement) -----

resource "aws_acm_certificate" "seer" {
  provider = aws.us_east_1

  domain_name       = local.seer_fqdn
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_route53_record" "seer_cert_validation" {
  for_each = {
    for dvo in aws_acm_certificate.seer.domain_validation_options : dvo.domain_name => {
      name   = dvo.resource_record_name
      record = dvo.resource_record_value
      type   = dvo.resource_record_type
    }
  }

  zone_id         = data.aws_route53_zone.root.zone_id
  name            = each.value.name
  type            = each.value.type
  records         = [each.value.record]
  ttl             = 60
  allow_overwrite = true
}

resource "aws_acm_certificate_validation" "seer" {
  provider = aws.us_east_1

  certificate_arn         = aws_acm_certificate.seer.arn
  validation_record_fqdns = [for r in aws_route53_record.seer_cert_validation : r.fqdn]
}

# ----- CloudFront distribution -----

resource "aws_cloudfront_distribution" "seer_static" {
  enabled             = true
  comment             = "seer (observability-first Bevy wasm) static bundle + perf reports"
  default_root_object = "index.html"
  price_class         = "PriceClass_100"

  aliases = [local.seer_fqdn]

  origin {
    domain_name              = aws_s3_bucket.seer_static.bucket_regional_domain_name
    origin_id                = "s3-seer-static"
    origin_access_control_id = aws_cloudfront_origin_access_control.seer_static.id
  }

  default_cache_behavior {
    target_origin_id       = "s3-seer-static"
    viewer_protocol_policy = "redirect-to-https"

    allowed_methods = ["GET", "HEAD"]
    cached_methods  = ["GET", "HEAD"]

    cache_policy_id            = data.aws_cloudfront_cache_policy.caching_optimized.id
    response_headers_policy_id = data.aws_cloudfront_response_headers_policy.simple_cors.id

    compress = true
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn      = aws_acm_certificate_validation.seer.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

# ----- DNS -----

resource "aws_route53_record" "seer" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = local.seer_fqdn
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.seer_static.domain_name
    zone_id                = aws_cloudfront_distribution.seer_static.hosted_zone_id
    evaluate_target_health = false
  }
}

# ----- GitHub Actions deploy role -----

resource "aws_iam_role" "seer_github_deploy" {
  name = "seer-github-deploy"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = {
        Federated = aws_iam_openid_connect_provider.github.arn
      }
      Action = "sts:AssumeRoleWithWebIdentity"
      Condition = {
        StringEquals = {
          "token.actions.githubusercontent.com:aud" = "sts.amazonaws.com"
        }
        StringLike = {
          "token.actions.githubusercontent.com:sub" = "repo:${var.github_repo}:*"
        }
      }
    }]
  })
}

resource "aws_iam_role_policy" "seer_github_deploy" {
  name = "deploy-permissions"
  role = aws_iam_role.seer_github_deploy.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "SeerBucketWrite"
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:DeleteObject",
          "s3:GetObject",
          "s3:ListBucket",
        ]
        Resource = [
          aws_s3_bucket.seer_static.arn,
          "${aws_s3_bucket.seer_static.arn}/*",
        ]
      },
      {
        Sid    = "SeerCloudFrontInvalidate"
        Effect = "Allow"
        Action = [
          "cloudfront:CreateInvalidation",
          "cloudfront:GetInvalidation",
        ]
        Resource = aws_cloudfront_distribution.seer_static.arn
      },
    ]
  })
}

output "seer_github_deploy_role_arn" {
  description = "Role ARN the seer deploy workflow assumes via OIDC."
  value       = aws_iam_role.seer_github_deploy.arn
}

output "seer_distribution_id" {
  description = "CloudFront distribution ID for seer — used by the workflow for invalidations."
  value       = aws_cloudfront_distribution.seer_static.id
}

output "seer_fqdn" {
  description = "Public hostname seer is served at."
  value       = local.seer_fqdn
}
