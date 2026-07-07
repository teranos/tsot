# game.sbvh.nl — static site for the game/ Bevy wasm bundle.
# Mirrors seer.tf — separate bucket + distribution + cert + role so
# game's blast radius is scoped to itself. Serves index.html +
# main.js + game.wasm at the root.

# ----- S3 bucket -----

resource "aws_s3_bucket" "game_static" {
  bucket        = var.game_bucket_name
  force_destroy = true
}

resource "aws_s3_bucket_public_access_block" "game_static" {
  bucket = aws_s3_bucket.game_static.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_cors_configuration" "game_static" {
  bucket = aws_s3_bucket.game_static.id

  cors_rule {
    allowed_origins = ["*"]
    allowed_methods = ["GET", "HEAD"]
    allowed_headers = ["*"]
    expose_headers  = []
    max_age_seconds = 3000
  }
}

resource "aws_cloudfront_origin_access_control" "game_static" {
  name                              = "${var.game_bucket_name}-oac"
  description                       = "OAC for game static bucket"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_s3_bucket_policy" "game_static_cf_read" {
  bucket = aws_s3_bucket.game_static.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid       = "AllowCloudFrontServicePrincipalRead"
        Effect    = "Allow"
        Principal = { Service = "cloudfront.amazonaws.com" }
        Action    = ["s3:GetObject"]
        Resource  = "${aws_s3_bucket.game_static.arn}/*"
        Condition = {
          StringEquals = {
            "AWS:SourceArn" = aws_cloudfront_distribution.game_static.arn
          }
        }
      }
    ]
  })
}

# ----- ACM certificate (us-east-1 — CloudFront requirement) -----

resource "aws_acm_certificate" "game" {
  provider = aws.us_east_1

  domain_name       = local.game_fqdn
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_route53_record" "game_cert_validation" {
  for_each = {
    for dvo in aws_acm_certificate.game.domain_validation_options : dvo.domain_name => {
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

resource "aws_acm_certificate_validation" "game" {
  provider = aws.us_east_1

  certificate_arn         = aws_acm_certificate.game.arn
  validation_record_fqdns = [for r in aws_route53_record.game_cert_validation : r.fqdn]
}

# ----- CloudFront distribution -----

resource "aws_cloudfront_distribution" "game_static" {
  enabled             = true
  comment             = "game (Bevy wasm) static bundle"
  default_root_object = "index.html"
  price_class         = "PriceClass_100"

  aliases = [local.game_fqdn]

  origin {
    domain_name              = aws_s3_bucket.game_static.bucket_regional_domain_name
    origin_id                = "s3-game-static"
    origin_access_control_id = aws_cloudfront_origin_access_control.game_static.id
  }

  default_cache_behavior {
    target_origin_id       = "s3-game-static"
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
    acm_certificate_arn      = aws_acm_certificate_validation.game.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

# ----- DNS -----

resource "aws_route53_record" "game" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = local.game_fqdn
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.game_static.domain_name
    zone_id                = aws_cloudfront_distribution.game_static.hosted_zone_id
    evaluate_target_health = false
  }
}

# ----- GitHub Actions deploy role -----

resource "aws_iam_role" "game_github_deploy" {
  name = "game-github-deploy"

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

resource "aws_iam_role_policy" "game_github_deploy" {
  name = "deploy-permissions"
  role = aws_iam_role.game_github_deploy.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "GameBucketWrite"
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:DeleteObject",
          "s3:GetObject",
          "s3:ListBucket",
        ]
        Resource = [
          aws_s3_bucket.game_static.arn,
          "${aws_s3_bucket.game_static.arn}/*",
        ]
      },
      {
        Sid    = "GameCloudFrontInvalidate"
        Effect = "Allow"
        Action = [
          "cloudfront:CreateInvalidation",
          "cloudfront:GetInvalidation",
        ]
        Resource = aws_cloudfront_distribution.game_static.arn
      },
    ]
  })
}

output "game_github_deploy_role_arn" {
  description = "Role ARN the game deploy workflow assumes via OIDC."
  value       = aws_iam_role.game_github_deploy.arn
}

output "game_distribution_id" {
  description = "CloudFront distribution ID for game — used by the workflow for invalidations."
  value       = aws_cloudfront_distribution.game_static.id
}

output "game_fqdn" {
  description = "Public hostname game is served at."
  value       = local.game_fqdn
}
